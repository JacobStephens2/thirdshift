# thirdshift

A factory that turns a GitHub issue into a ready-for-review pull request by running unattended agent sessions against it. The humans are the day shift; the agents work the third shift, overnight, on any server, while you do something else.

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
6. Keeps the pull request mergeable and green: merges the Base branch in and watches CI, starting a **Repair** session for a merge conflict or failing checks, at most 3 per Run. If the Base branch moves while CI runs, it merges it again and goes round, at most 3 times per Run.
7. Cleans up: removes the worktree, the local Issue branch and the temporary plugin directory, whether the Run succeeded or not. The one exception is a **Failed run** whose work could not be pushed: see below.

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

- **stdout** carries only the pull request's URL: on success, and on a Failed run that leaves a draft pull request. The exit code tells the two apart, so script it as `url=$(thirdshift "$issue") && echo "ready: $url"`.
- **stderr** carries everything else: errors, cleanup problems, and progress lines while sessions run. A successful Run's last line names the pull request too.
- **Exit code** `0` means the Run ended with a pull request the factory stands behind. `2` means the argument is missing or isn't a GitHub Issue URL; the error and the help text go to stderr. Any other failure exits `1`.

The other commands:

```sh
thirdshift update    # update to the latest release (see Updating)
thirdshift version   # print thirdshift <version>
thirdshift help      # print every form of the command, each with a one-line description
```

`version` and `help` print to stdout and exit `0`. `update` follows the Run's rule: stdout stays empty, messages go to stderr.

Uncommitted changes in your clone are fine: the Run works in its own worktree from `origin`, so they are simply left out. Unpushed commits on the Base branch are not: push them first, or the Run stops.

### Logs

Each session's full transcript, as Claude Code's `stream-json` output, is written to its own file under `~/.thirdshift/logs/` (created if missing):

```
~/.thirdshift/logs/<owner>-<repo>-issue-<n>-<timestamp>-implement.jsonl
~/.thirdshift/logs/<owner>-<repo>-issue-<n>-<timestamp>-repair-<i>.jsonl
```

A Resume is logged as its session's kind plus `-resume`, e.g. `implement-resume.jsonl`.

All sessions in a Run share the Run's UTC timestamp, so a Run's logs sort together. When a Run fails, stderr ends with the path of its most recent session log, the place to start looking.

## Continuation

Running thirdshift again on an issue picks up where the last Run stopped ([ADR-0002](docs/adr/0002-existing-issue-branch-means-continue.md)). It looks at the highest-numbered Issue branch:

- **No Issue branch yet**: a fresh Run creates `issue-<n>` from the Base branch.
- **The branch exists with no pull request, or an open one**: the Run is a **Continuation**. It checks out that branch, and the agent builds on its commits instead of starting over, then creates the pull request or updates the open one. With an open pull request, that pull request's base is the Base branch, whatever you have checked out.
- **The branch's pull request was merged or closed**: finished work is never reopened. The Run starts fresh on the next number, `issue-<n>-branch-2`, `-3`, and so on. A number counts as used even if GitHub deleted the branch after merging.

So you can retry a Failed run with the same command, or start an issue on one server and continue it on another.

A Run is not idempotent: re-running builds on whatever is already on the branch, including a Failed run's work-in-progress commit.

## Failed runs

A **Failed run** is one that ends, including by Ctrl-C or a closed terminal, without an open pull request from its Issue branch that targets the Base branch, is mergeable and has passing CI. Causes include the session exiting non-zero, no pull request or one with the wrong base, running out of Repairs, a session that still ends while waiting on background work after its Resume, and a Base branch that keeps moving while CI runs.

A Failed run:

1. Commits any uncommitted work as `thirdshift: failed run (<reason>)`, with a timestamp and the hostname, and pushes the Issue branch, so nothing is lost. If the branch has no changes against the Base branch, nothing is pushed.
2. Converts its open pull request, if any, back to a draft, so a pull request only claims to be ready when the factory stands behind it. The next successful Continuation marks it ready again.
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

To install it system-wide instead, build it and copy it into place:

```sh
cargo build --release
sudo install -m 755 target/release/thirdshift /usr/local/bin/thirdshift
```

Editing a skill in `skills/` has no effect until you rebuild and reinstall ([ADR-0001](docs/adr/0001-rust-binary-with-embedded-skills.md)).

Running the test suite (`cargo test`) also needs **`python3`** on `PATH`: the integration tests swap in fake `gh` and `claude`, which are Python scripts in `tests/fakes/`.

## Releasing

1. Bump `version` in `Cargo.toml` (and `Cargo.lock`) in a pull request.
2. Merge it.
3. Push a `v<version>` tag on the merge commit, e.g. `git tag v0.2.0 && git push origin v0.2.0`.

The tag starts the release workflow, generated by [`dist`](https://github.com/axodotdev/cargo-dist) from `dist-workspace.toml`. It builds the Linux and macOS binaries and the installer, checks that the Linux binary starts on Rocky Linux 9 and in WSL2 with Ubuntu 24.04, and only then publishes the GitHub Release. It then publishes the same version to crates.io, skipping it if it is already there, and finally replaces the Release's body with notes generated from the merged pull requests.

## Credits and license

thirdshift is released under the [MIT License](LICENSE).

The Factory skills in `skills/` are adapted from Matt Pocock's [mattpocock/skills](https://github.com/mattpocock/skills), changed to run headless with no human in the loop. They are used under the MIT License; its copyright notice is kept in [`skills/LICENSE`](skills/LICENSE) and is embedded in the binary and written out with the skills on every Run. The `pr` skill also reproduces part of Dex Horthy's `show-me` skill; see [`skills/pr/CREDITS.md`](skills/pr/CREDITS.md).
