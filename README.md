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
4. Runs a headless Claude Code session in that worktree with the **Factory skills** in [`skills/`](skills/) loaded, started with one of the **Session prompts** in [`prompts/`](prompts/). The agent implements the issue, reviews its work against the Base branch, addresses the **Standards findings** and **Spec findings** it agrees with, and opens a ready-for-review pull request that lists every **Unaddressed finding** with a reason and closes the issue.
5. Takes over deterministically: pushes the Issue branch, skipping your repo's git hooks since the session runs the tests and CI gates the pull request, checks through `gh` that the pull request exists, is open and targets the Base branch, and marks it ready for review (`gh pr ready`) if the agent left it as a draft.
6. Keeps the pull request mergeable and green: merges the Base branch in and watches CI, starting a **Repair** session for a merge conflict or failing checks, at most 5 per Run. A failed check that also failed, under the same name, on the Base branch commit the head last merged in is an **Inherited failure**, not the branch's to fix: the CI-fix Repair is given only the branch's own failures, with the Inherited failures listed as not to fix. If every failed check is an Inherited failure, no Repair starts: the Run merges the Base branch again if it has moved since, and otherwise ends as a [Failed run](#failed-runs) that says to fix the Base branch first, unless it was given `base-fix` or the [User config](#user-config) sets `base.fix`, when it starts a [Base fix](#base-fix) first. A check that is pending, passed or missing on that Base branch commit is the branch's own, and thirdshift never triggers or waits for the Base branch's CI. If a CI-fix Repair leaves the head unchanged, with no commit of its own and no Base branch move to merge, thirdshift does a **Check re-run**, once for that head: it asks GitHub to re-run the branch's own failed checks (`gh run rerun <run-id> --failed`, once per workflow run), says so on stderr as `re-running the failed checks on <short sha>: <check>[, <check>…]`, and watches CI on that head again, from the new attempt on. A check that failed for a reason that does not repeat, such as a flaky test or a runner fault, then passes, and the Run goes on as for any green head. If CI is red again, the Run ends as a Failed run, with no second Repair or re-run for that head. It ends the same way, with nothing re-run, if one of those checks can't be re-run (a commit status, or a check run that is not a GitHub Actions job), since the head could not go green, or if GitHub refuses the re-run, which stderr shows. What the Repair concluded is never read, Inherited failures are never re-run on their own account, and a Check re-run is no Repair, so it counts against no budget ([ADR-0009](docs/adr/0009-one-check-re-run-before-a-declined-ci-fix.md)). If the Base branch moves while CI runs, it merges it again and goes round, at most 5 times per Run. In a Merge run, each round also takes in **Foreign commits** first: see [Foreign commits in a Merge run](#foreign-commits-in-a-merge-run).
7. In a **Merge run** (`thirdshift merge <Issue URL>`), does the **Self-merge**: once the pull request is open, ready for review, mergeable and green, thirdshift merges it into the Base branch with a merge commit, on exactly the head commit whose CI it watched (`gh pr merge --merge --match-head-commit <sha>`). It never uses GitHub's auto-merge ([ADR-0004](docs/adr/0004-self-merge-by-thirdshift-not-github-auto-merge.md)). A merge that fails goes back round step 6, within the same budgets, and is tried again on the new green head. If that round finds nothing to fix, the refusal is a **policy refusal**, such as merge commits being disallowed or a review being required. After the merge, thirdshift deletes the Issue branch on `origin`, and closes the issue if it is still open, with the comment `Closed by #<pr>, merged into <base> by a thirdshift Merge run.` GitHub closes it on its own only for a merge into the repository's default branch, and then thirdshift leaves it alone.
8. Cleans up: removes the worktree, the local Issue branch and the temporary plugin directory, whether the Run succeeded or not. The one exception is a **Failed run** whose work could not be pushed: see below.

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
- **stderr** carries everything else: errors, cleanup problems, and progress lines while sessions run. Each line starts with the local time it was printed, as in `thirdshift: 12:14:49 pushing issue-7`, so a quiet terminal shows how long the Run has been on its last step. A successful Run's last line names the pull request too: `PR <url> is ready for review`, or `PR <url> is merged` after a Merge run, followed only by a `warning:` line if a [Run notification](#run-notifications) can't be sent.
- **Exit code** `0` means the Run ended with a pull request the factory stands behind, merged in a Merge run. Once the Self-merge has merged, the Run succeeds even if deleting the Issue branch on `origin` or closing the issue then fails: the merge can't be undone, so each failed step is a `warning:` line on stderr naming the command to run by hand, and the Run still exits `0` with the URL on stdout. Ctrl-C likewise: before the merge it makes a Failed run, after it thirdshift finishes these steps and exits as merged. `2` means the command is one thirdshift can't use: the Issue URL is missing or isn't a GitHub Issue URL, there is an argument other than the URL and the Run flags, a Run flag is repeated or contradicts another, or `parallel` isn't followed by a whole number from 1 up; the error and the help text go to stderr, before any work. A [User config](#user-config) thirdshift can't use exits `1`, also before any work. Any other failure exits `1`.

The other commands:

```sh
thirdshift architect [<focus>]      # review the Base branch's architecture, publish a plan and run it (see Architect runs)
thirdshift architect [<focus>] --plan-only   # publish the plan, mark it ready and stop there
thirdshift email-test [<address>]   # send a test email through Resend (see Email)
thirdshift setup                    # choose your defaults and write the User config with every setting (see User config)
thirdshift update                   # update to the latest release (see Updating)
thirdshift version                  # print thirdshift <version>
thirdshift help                     # print every form of the command, each with a one-line description
```

`version` and `help` print to stdout and exit `0`. `update`, `setup` and `email-test` follow the Run's rule: stdout stays empty, messages go to stderr. `architect` follows it too: stdout carries the pull request's URL of the run it dispatches, or, in its place, the URL of the issue it ended on: its plan with `--plan-only`, or its idea when the review published no plan.

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
dir = "~/elsewhere/logs"   # where session logs go, instead of ~/.thirdshift/logs

[spec]
parallel = 2   # how many Tickets a Spec run runs at once, instead of 3
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
dir = "~/.thirdshift/logs"   # where session logs go; default ~/.thirdshift/logs

[spec]
parallel = 3   # how many Tickets a Spec run runs at once; default 3
```

Every key holds its real value, so a Run reading it does exactly what it does with no file. `email.to` has no default, so `setup` suggests one: the public email of your GitHub profile (from `gh api user`), else your global git `user.email`, unless that is a `@users.noreply.github.com` address, which can't receive mail. With neither, `email.to` is the only line written commented out, as above. `setup` never asks `gh` for more scopes, so a private GitHub email is not read, and a Run never looks the suggestion up: `--email` with no address and no `email.to` still stops the Run. From a terminal (stdin and stderr both terminals), `setup` first asks, on stderr:

1. Every Run a Merge run? (`merge.always`)
2. With that on, every Run may start a Base fix when the Base branch's CI is red? (`base.fix`). With it off, this is not asked, and `base.fix` is written at its default, `false`.
3. Every Run first fast-forwards your checkout of the Base branch? (`launch.pull`)
4. Run notifications? (`email.always`). If yes, the address (`email.to`), asked again until it has an `@`, then the sender (`email.from`).
5. With notifications on, the Resend API key, with input hidden. With none saved it asks `Resend API key (input hidden, Enter to skip):`; with one in the [Credentials](#email), `Resend API key (input hidden, Enter keeps the saved one):`, and a new one replaces it. Surrounding spaces are trimmed, and anything that doesn't start with `re_` is asked again. A key you give is saved in the Credentials, `~/.thirdshift/credentials.toml`, created with mode 600 (and `~/.thirdshift` with it) or edited in place, keeping its comments and anything else in it and changing only `resend.key`; `setup` then prints `wrote the Credentials <path>`. The key is never printed, nor written to the User config. Skipping writes no Credentials and says how to add a key later: rerun `thirdshift setup`, or set `RESEND_API_KEY`. With `RESEND_API_KEY` set and not empty, which wins over the Credentials, nothing is asked, and it says the key comes from `RESEND_API_KEY`. With a key found or given, it offers to send a test email (default No), as `thirdshift email-test` does, once the files are written.

Pressing Enter takes the default shown, which is the file's current value, or else the setting's default, and for the address the suggested email above. `logs.dir` is not asked about. The answers are written like everything else below: in place, keeping your comments. Ctrl-C during the questions, the key included, writes nothing: neither the User config nor the Credentials. With notifications off, nothing about a key is asked, and saved Credentials stay as they were, so `--email` on a single Run still works. Credentials a Run would refuse (see [Email](#email)) are refused before any question, exit `1`, and not touched. With no terminal, as from cron or `thirdshift setup </dev/null`, `setup` asks nothing and never writes the Credentials. Either way it prints the file's path on stderr and exits `0` with stdout empty. Over a User config that is already there, `setup` edits it in place: its comments and key order stay, as do the values it didn't ask about, and each key it lacks is added at its default with its comment, so afterwards the file lists every setting this version knows. One that already does, down to the commented-out `email.to` line, is left byte for byte as it was. A key added to an inline table, such as `launch = { pull = true }`, gets no comment, since TOML has no place for one there. One a Run would refuse is refused the same way, exit `1`, and not touched. Any argument after `setup` is an argument error (exit `2`).

The first Run on a machine with no User config, started from a terminal, offers Setup before any work, on stderr: `No User config at <path>. Set your defaults now? [Y/n]`. Yes (or Enter) asks the questions above, writes the file, and the Run carries on using your answers; a flag in the command, such as `--no-merge`, `--email` or `--no-email`, still wins over them. No writes every setting at its default, as `setup` with no terminal does, asks nothing about a key, writes no Credentials, says that `thirdshift setup` changes it, and the Run carries on; later Runs find the file and don't offer again. If the file can't be written, stderr gets a `warning:` line and the Run carries on with the defaults. Ctrl-C during the offer or the questions writes nothing and ends the command before any work, with no Run notification. A command thirdshift can't parse exits `2` before any offer. A Run with no terminal, as from cron, CI, `nohup` or an agent's shell, offers nothing, writes nothing, and runs on the defaults, so a later Run from a terminal still gets the offer.

With `merge.always = true`, `thirdshift <Issue URL>` is a Merge run, and `thirdshift --no-merge <Issue URL>` (or `no-merge`, before or after the URL) leaves that one Run's pull request ready for review.

With `base.fix = true`, every Run may start a [Base fix](#base-fix), as if given `base-fix`, and `thirdshift --no-base-fix <Issue URL>` (or `no-base-fix`, before or after the URL) forbids it for that one Run.

With `launch.pull = true`, every Run brings the Base branch checked out in the directory you start it from (the **Launch directory**) up to date with `origin`, so you no longer `git pull` by hand before each Run. It happens after the pre-flight checks pass and before the worktree is created, as `git merge --ff-only origin/<Base branch>`: fast-forward only, never a merge commit or a rebase, always from `origin`, whatever the branch's upstream or your `pull.*` settings. A progress line on stderr says when it updates the branch; an already up-to-date branch is left quietly as it is. It is skipped when the checked-out branch isn't the Base branch, as in a Continuation whose open pull request targets another base, or on a detached HEAD. If the update can't happen, for example because uncommitted changes are in the way, stderr gets a `warning:` line with git's error and the command to run by hand, your changes are left as they were, and the Run carries on with the same outcome and exit code. The setting only affects your checkout: the Run's worktree starts from `origin/<Base branch>` either way.

With `email.always = true`, every Run sends a [Run notification](#run-notifications) to `email.to`, as if given `--email`, and `thirdshift --no-email <Issue URL>` (or `no-email`, before or after the URL) sends none for that one Run.

`spec.parallel` sets how many Tickets a [Spec run](#spec-runs) runs at once, by default 3. It must be a whole number from 1 up; `parallel <n>` on the command line wins over it for one Spec run.

`logs.dir` sets the directory [session logs](#logs) are written to, created if missing. It must be an absolute path, `~` or a path starting with `~/`, where `~` stands for `$HOME`. A relative path stops the Run before any work, since the directory a Run is launched from is no base for a setting that holds for every Run.

A Run reads the file before any work. One that isn't valid TOML, or that has a key or section thirdshift doesn't know, such as `alway` for `always`, or a value of the wrong type, such as anything but `true` or `false` for `always`, or `0` for `spec.parallel`, stops the Run with an error naming the file and the offending key, so a typo can't silently leave a setting off. `email-test` and `setup` read it the same way. `update`, `version` and `help` never read it, so a broken User config can't block them, and they never offer Setup; nor does `email-test`.

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

The Credentials are read only when `RESEND_API_KEY` is unset or empty, and only by `email-test`, `setup`, and a Run or an Architect run that asks for a notification. A missing file just means no key from it. One that isn't valid TOML, or holds anything but a quoted `resend.key`, such as a typo like `kye`, stops the command with exit `1`, naming the file and the offending key, and nothing is sent. One that others can read is still used, with a `warning:` line on stderr saying to `chmod 600` it. Nothing is sent to check the key itself. When Resend accepts the email, it prints `accepted by Resend; check your inbox` and exits `0`; that is all it can verify, so check that the email arrives. When Resend refuses it, for example for a bad key or a sender it won't send from, it prints Resend's error text word for word, with where the key came from, and exits `1`. It gives up after 30 seconds without an answer.

#### Run notifications

`--email` (or `email`) asks a Run for a **Run notification**: one email, sent when the Run ends, whatever the outcome. The word after the flag is the address only if it contains `@` and doesn't start with `https://`, so the Issue URL is never taken for it; otherwise the email goes to `email.to`. With `email.always = true` in the [User config](#user-config), a Run asks for one without the flag, and `--no-email` (or `no-email`) skips it for that Run; an address after `--email` still wins over `email.to`. Giving a flag twice, or `--email` together with `--no-email`, is an argument error.

A Run that asks for a notification, by the flag or by `email.always`, makes the same checks as `email-test` before any other work: an address is known, and a key is found, in `RESEND_API_KEY` or else the Credentials. If either fails, or the Credentials are broken, the Run stops, exits `1` naming what's wrong (with no key, the message above listing every way to give one), and sends nothing. A Run that asks for no notification never reads the Credentials, so a broken file can't stop it, and `update`, `version` and `help` never read it either. Once they pass, every way the Run ends sends exactly one notification, after its outcome is final and its cleanup done: ready for review, merged, a [Failed run](#failed-runs) (including a later preflight failure such as an origin mismatch), or interrupted by Ctrl-C, SIGTERM or a closed terminal.

- **Subject**: `[thirdshift] <owner>/<repo>#<n> <issue title>: <outcome>`, where the outcome is `ready for review`, `merged`, `failed` or `interrupted`. The title is left out if it can't be read from GitHub.
- **Body**, plain text: the pull request URL (if any), the failure cause (if failed), the session log path (if any), the hostname and how long the Run took.

A [Spec run](#spec-runs) sends at most one notification for the whole Spec, under the same rules, with its checks made once before any Ticket starts. Its subject names the Spec, its outcome is the Spec run's, and its body, after what a Run's holds (the Spec PR, if any), lists each Ticket's outcome, one line per Ticket as in the summary on stderr, such as `#21 landed with https://github.com/acme/widgets/pull/1` or `#22 blocked by #21`. The Ticket Runs inside it never send a notification of their own, whatever the User config says.

An [Architect run](#architect-runs) takes the same flags and the same `email.always`, and sends [one notification](#one-run-notification) covering its review and the run it dispatched.

A notification that can't be sent is a `warning:` line on stderr with Resend's error. It never changes the Run's outcome, stdout or exit code.

### Logs

Each session's full transcript, as Claude Code's `stream-json` output, is written to its own file under `~/.thirdshift/logs/`, or the `logs.dir` set in the [User config](#user-config) (created if missing):

```
~/.thirdshift/logs/<owner>-<repo>-issue-<n>-<timestamp>-implement.jsonl
~/.thirdshift/logs/<owner>-<repo>-issue-<n>-<timestamp>-repair-<i>.jsonl
~/.thirdshift/logs/<owner>-<repo>-architect-<timestamp>-architecture-review.jsonl
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

## Base fix

A Base branch whose CI is broken fails every Run on it with an Inherited failure. To have a Run fix the Base branch itself and carry on, give it `base-fix` (or `--base-fix`), before or after the URL:

```sh
thirdshift base-fix https://github.com/acme/widgets/issues/7
```

To allow it for every Run on a machine, set `base.fix` in the [User config](#user-config); `no-base-fix` (or `--no-base-fix`), before or after the URL, then forbids it for one Run. Giving either flag twice, or `base-fix` together with `no-base-fix`, is an argument error.

When the Run's only red checks are Inherited failures and the Base branch has not moved since, a Run allowed to then starts a **Base fix**, at most one per Run ([ADR-0008](docs/adr/0008-inherited-failures-fail-the-run.md)):

1. thirdshift looks for an open issue labelled `base-fix`, other than the Run's own, whose title names the same Base branch and every one of those checks. If there is one, another Run's Base fix is already under way, and the Run waits on that one instead: see [One Base fix per Base branch](#one-base-fix-per-base-branch).
2. Otherwise, thirdshift opens an issue from a fixed template, with no agent session: titled `CI red on <base>: <check>[, <check>…]`, its body naming the checks with their URLs on the Base branch, the Base branch and its short sha, and the Run's pull request, labelled `base-fix` and `ready-for-agent`. A label the repository lacks is added to it first.
3. It starts a child `thirdshift` on that issue from the same Launch directory, as a Merge run into the Run's Base branch, whatever the Run's own goal. Its progress lines are relayed with a `#<n>: ` prefix, after `starting Base fix #<n> into <base>: <issue URL>` and `waiting on Base fix #<n>`. The Base fix sends no Run notification, treats every red check as its own to fix rather than as an Inherited failure, and never starts a Base fix of its own.
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

Without `base-fix` or `base.fix`, or with `no-base-fix`, an Inherited failure fails the Run as described in [What a Run does](#what-a-run-does).

## Continuation

Running thirdshift again on an issue picks up where the last Run stopped ([ADR-0002](docs/adr/0002-existing-issue-branch-means-continue.md)). It looks at the highest-numbered Issue branch:

- **No Issue branch yet**: a fresh Run creates `issue-<n>` from the Base branch.
- **The branch exists with no pull request, or an open one**: the Run is a **Continuation**. It checks out that branch, and the agent builds on its commits instead of starting over, then creates the pull request or updates the open one. With an open pull request, that pull request's base is the Base branch, whatever you have checked out.
- **The branch's pull request was merged or closed**: finished work is never reopened. The Run starts fresh on the next number, `issue-<n>-branch-2`, `-3`, and so on. A number counts as used even if GitHub deleted the branch after merging.

So you can retry a Failed run with the same command, or start an issue on one server and continue it on another.

A Run is not idempotent: re-running builds on whatever is already on the branch, including a Failed run's work-in-progress commit.

## Failed runs

A **Failed run** is one that ends, including by Ctrl-C or a closed terminal, without an open pull request from its Issue branch that targets the Base branch, is mergeable and has passing CI, or, for a Merge run, without that pull request merged. Causes include the session exiting non-zero, no pull request or one with the wrong base, running out of Repairs, CI red only on **Inherited failures** (`CI red on <check>[, <check>…], which also fails on <base> at <short sha>; fix <base> first`), a CI-fix Repair that finds nothing on the branch to fix (a **declined CI fix**: the head is unchanged after it, and its one Check re-run left CI red or could not happen), a session that still ends while waiting on background work after its Resume, and a Base branch that keeps moving while CI runs, or in a Merge run, an Issue branch that keeps getting Foreign commits.

A Failed run:

1. Commits any uncommitted work as `thirdshift: failed run (<reason>)`, with a timestamp and the hostname, and pushes the Issue branch, so nothing is lost. If the branch has no changes against the Base branch, nothing is pushed.
2. Converts its open pull request, if any, back to a draft, so a pull request only claims to be ready when the factory stands behind it. The next successful Continuation marks it ready again. The exception is a Merge run's policy refusal: the pull request is ready, mergeable and green and only the Self-merge could not happen, so it stays ready for review, and no failure commit is pushed onto the head whose CI was watched.
3. Cleans up as usual, prints the reason to stderr and exits non-zero. If the push failed, the worktree and local Issue branch are kept instead, and stderr names the branch, its head commit and the worktree path, so you can recover the work or push it by hand.

Merges, never rebases or force-pushes: a branch worked on from several servers never loses history.

## Spec runs

A **Spec** is an issue with sub-issues, its **Tickets**. `thirdshift <Issue URL>` on a Spec is a **Spec run**: it works through the Tickets in the order their GitHub "blocked by" links allow, each Ticket's **Run** a **Merge run** into the **Spec branch**, then leaves one **Spec PR** from the Spec branch into the Base branch ready for review ([ADR-0006](docs/adr/0006-spec-runs-merge-tickets-into-a-spec-branch.md)). An issue with no sub-issues is an ordinary Run.

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
thirdshift: 03:12:40 #21 failed: claude exited 1 (session log: ~/.thirdshift/logs/acme-widgets-issue-21-….jsonl)
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

`thirdshift architect` starts an **Architect run**: the factory looks for architecture work itself, with no **Issue URL**, publishes a plan for the top opportunity, and implements it. Start it from a clone of the repository, on the **Base branch**:

```sh
cd ~/repos/widgets
thirdshift architect                              # review the whole codebase, then run the plan
thirdshift architect "the Spec run"               # point the review at an area
thirdshift architect merge parallel 2             # merge the plan's pull request, two Tickets at once
thirdshift architect base-fix                     # the run the plan is dispatched as may start a Base fix
thirdshift architect "the Spec run" --plan-only   # publish the plan, mark it ready and stop
thirdshift architect --email you@example.com      # email how the Architect run ended
```

The focus is optional free text, given as one argument, anywhere among the flags; it goes into the Session prompt. `architect` is a command only as the first argument.

`merge`, `no-merge`, `base-fix`, `no-base-fix` and `parallel <n>` (or `--merge`, `--no-merge`, `--base-fix`, `--no-base-fix` and `--parallel <n>`) are for the run the plan is dispatched as, and mean what they do for `thirdshift <Issue URL>`, so none of them is ever read as the focus. `--plan-only` stops the Architect run once the plan is marked ready, and dispatches nothing. `email`, optionally followed by an address, and `no-email` (or `--email` and `--no-email`) are for the Architect run's own [Run notification](#one-run-notification), with or without `--plan-only`. The word after `email` is the address only if it contains `@`, so a focus without one is never taken for it.

A second focus, a repeated flag, `merge` with `no-merge`, `base-fix` with `no-base-fix`, `email` with `no-email`, `parallel` without a whole number from 1 up, any other argument starting with a dash, or an empty focus is an argument error (exit `2`). So is `merge`, `no-merge`, `base-fix`, `no-base-fix` or `parallel` with `--plan-only`, since nothing is dispatched for them to apply to.

An Architect run:

1. Makes the checks a Run makes that don't need an issue, before creating anything: `origin` is a GitHub repository, git has a `user.name` and `user.email`, HEAD is not detached, and the Base branch exists on `origin` with your local copy not ahead of it. There is no **Origin match**, since there is no Issue URL: the repository is the one `origin` names. With `launch.pull = true` in the [User config](#user-config), it then fast-forwards your checkout of the Base branch, as a Run does.
2. Creates a git worktree next to your clone, named `<repo>-architect`, detached at the head of the Base branch on `origin`, with no **Issue branch**. Your checkout, its uncommitted changes and its untracked files are never touched or scanned.
3. Runs the **Architecture review** there: a headless Claude Code session with the **Factory skills** loaded, started with the [Architecture review prompt](prompts/architecture-review.md). It looks for deepening opportunities, skips any an open issue already covers, and takes the top recommendation. If it is Strong, the review publishes it as the plan, labelled `needs-triage`: a **Spec** with **Tickets**, or a single Ticket when one session is enough. It may edit files in the worktree to check an idea, but commits and pushes nothing. It ends its final message with one line naming the plan: `Architecture review plan: <Issue URL>`. thirdshift reads only that line. With [no Strong candidate](#no-strong-candidate) the line names another issue, and steps 5 to 7 are skipped.
4. Removes the worktree and the temporary plugin directory once the session ends, whatever the outcome.
5. Checks the plan: the issue is in this repository, is open, was created after the Architect run started, and carries no other label that says it is not agent work (`ready-for-human`, `needs-info` or `wontfix`).
6. Marks the plan ready, in one request: `needs-triage` is swapped for `ready-for-agent`, and its other labels are kept. A Spec's Tickets are left as the review labelled them.
7. Dispatches the plan, unless given `--plan-only`, exactly as `thirdshift <plan URL>` would from the same clone: a [Spec run](#spec-runs) when the plan has sub-issues, a Run otherwise. `merge` and `no-merge` apply to the Spec PR or the Run's pull request, `parallel <n>` to the Spec run, and `base-fix` and `no-base-fix` to whether that run may start a [Base fix](#base-fix), as if given to that command, and the [User config](#user-config) sets what they leave unsaid: `merge.always`, `spec.parallel`, `base.fix`, `launch.pull` and `logs.dir`. The Architecture review itself never watches CI, so a Base fix can only happen in the dispatched run. The one difference is that it sends no [Run notification](#run-notifications) of its own, whatever `email.always` says: the Architect run sends [the one](#one-run-notification).

The dispatched run's ending is the Architect run's: its exit code, its pull request's URL alone on stdout, and its last line on stderr, `PR <url> is ready for review` or `PR <url> is merged`. If it fails, the Architect run fails as that Failed run or Failed spec run does, with the cause and the session log on stderr and the pull request's URL on stdout if it left one. The plan stays `ready-for-agent`, for `thirdshift <plan URL>` to take up again. `parallel <n>` on a plan that is a single Ticket fails the same way as it does for `thirdshift <Issue URL>` on an issue that isn't a Spec: `parallel is only for a Spec, and #<n> has no sub-issues`, exit `1`, before any implementing, with the plan left ready to run without it.

With `--plan-only`, it instead exits `0` with the plan's URL alone on stdout, and `plan <url> is ready for an agent` as stderr's last line. Read or edit the plan, then run it with `thirdshift <Issue URL>`.

Progress lines on stderr say when the review starts, which plan it reported, when the labels are swapped, and when the plan is dispatched: `dispatching the plan <url>, as thirdshift <url> would`. The dispatched run's own progress lines follow. With no Strong candidate, the last line says which issue the Architect run ended on instead.

A review session that fails or is interrupted, a final message without one of the lines the prompt asks for, or a plan that fails a check ends the Architect run as a failure: exit `1`, nothing on stdout, the cause on stderr and then the path of the session log. Nothing is dispatched and no label is changed, so a plan the review did publish stays `needs-triage` for you to finish or close. A review that finds no deepening opportunity at all has no issue to name, so it ends without one of those lines and the Architect run fails this way too; its session log says what it looked at.

### One Run notification

An Architect run asked for a [Run notification](#run-notifications), by `email` or by `email.always = true` in the [User config](#user-config) without `no-email`, sends exactly one, whatever its outcome and with or without `--plan-only`. It makes a Run's checks before any other work, an address and a Resend API key, and stops with exit `1` if either is missing. The email goes after the outcome is final and printed, and a failed send is only a `warning:` line on stderr: it changes neither the exit code nor stdout.

- **Subject**: `[thirdshift] <owner>/<repo> Architect run: <outcome>`. With a dispatched run, the outcome is that run's: `ready for review`, `merged`, `failed` or `interrupted`. Without one, it is the review's: `plan published` (with `--plan-only`), `idea filed`, `idea already filed`, `review failed` (also for a plan that fails a check) or `interrupted`. The repository is left out if `origin` doesn't name one on GitHub.
- **Body**, plain text: a `Review:` line saying how the Architecture review ended, with the URL of the plan or idea issue it named (`plan published: <url>`, `idea filed: <url>`, `idea already filed: <url>`, `failed` or `interrupted`); when the plan was dispatched, a `Dispatched:` line with that run's outcome; then what a Run's notification holds, for the dispatched run or else the failed review: the pull request URL (if any), the failure cause (if failed), a `Base fix:` line for a dispatched run that started or waited on a [Base fix](#base-fix), the session log path (if any), the hostname and how long the Architect run took. After a dispatched Spec run, it ends with a line per Ticket, as a Spec run's notification does.

### No Strong candidate

Only a Strong top recommendation becomes a plan. When the review's top recommendation is Worth exploring or Speculative, it publishes no plan, and the Architect run ends in one of two ways, both a success, with or without `--plan-only`: exit `0`, with one issue's URL alone on stdout. thirdshift changes no label on that issue and dispatches nothing: there is nothing to run.

- **An idea issue.** The review files its top recommendation as one issue labelled `needs-triage`, for the **Day shift** to flesh out, and ends its final message with `Architecture review idea: <Issue URL>`. stdout carries the idea issue's URL, and stderr's last line is `no Strong candidate: the Architecture review filed the idea <url>`.
- **Already filed.** An open issue already covers that recommendation, so the review files nothing and ends its final message with `Architecture review already filed: <Issue URL>`. stdout carries that issue's URL, and stderr's last line is `no Strong candidate: <url> already covers the Architecture review's top recommendation, so it filed nothing`.

Start one Architect run per repository at a time. Nothing stops a second one, but two at once may pick the same opportunity and publish the same plan, and the second can't create its worktree while the first's is there.

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

The Factory skills in `skills/` are adapted from Matt Pocock's [mattpocock/skills](https://github.com/mattpocock/skills), changed to run headless with no human in the loop. They are used under the MIT License; its copyright notice is kept in [`skills/LICENSE`](skills/LICENSE) and is embedded in the binary and written out with the skills on every Run. The `pr` skill also reproduces part of Dex Horthy's `show-me` skill; see [`skills/pr/CREDITS.md`](skills/pr/CREDITS.md).
