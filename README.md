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
5. Takes over deterministically: pushes the Issue branch, checks through `gh` that the pull request exists, is open and targets the Base branch, and marks it ready for review (`gh pr ready`) if the agent left it as a draft.
6. Keeps the pull request mergeable and green: merges the Base branch in and watches CI, starting a **Repair** session for a merge conflict or failing checks, at most 3 per Run.
7. Cleans up: removes the worktree, the local Issue branch and the temporary plugin directory, whether the Run succeeded or not.

### Status

The walking skeleton works: a fresh Run on the happy path, with the Origin match, the implement session, the push, the pull request checks (except marking a draft ready), and cleanup. The rest is tracked in the [Spec issue #2](https://github.com/JacobStephens2/thirdshift/issues/2) and its sub-issues: pre-flight checks, **Continuation**, numbered Issue branches, the Failed run path, **Repairs**, and progress lines on stderr. This README describes the designed behaviour.

## Install

thirdshift is a single Rust binary with the Factory skills compiled in, so it runs without a checkout of this repository:

```sh
cargo install --path .
```

Editing a skill in `skills/` has no effect until you rebuild and reinstall ([ADR-0001](docs/adr/0001-rust-binary-with-embedded-skills.md)).

### Updating

```sh
thirdshift update
```

It replaces the installed binary with the latest stable GitHub Release, or says it is already on it. Messages go to stderr and stdout stays empty. It exits `0` when it updated or was already up to date, and `1` on any failure, such as no network. Updating is safe while a Run is using the old binary, and a Run never checks for updates or updates itself.

`thirdshift update` only replaces a copy put in place by the release's shell installer, which leaves an install receipt in `~/.config/thirdshift/`. It refuses to touch any other copy and names the command that updates it. For a build from source, pull and reinstall:

```sh
git checkout main && git pull
cargo install --path .
```

`cargo install` puts the binary in `~/.cargo/bin`. To install it system-wide instead, build it and copy it into place:

```sh
cargo build --release
sudo install -m 755 target/release/thirdshift /usr/local/bin/thirdshift
```

Keep a single copy: `~/.cargo/bin` usually comes before `/usr/local/bin` on `PATH`, so with a copy in each you can end up running a stale one without noticing. `type -a thirdshift` lists every copy on your `PATH`.

## Prerequisites

The Linux user that runs thirdshift needs:

- **`claude`** (Claude Code), logged in.
- **`gh`** (GitHub CLI), logged in.
- **`git`** with a global `user.name` and `user.email`, and credentials that can push to the repository (`gh auth setup-git` makes git use `gh`'s login). The agents commit as this identity; without it, an agent may borrow the author of the last commit.
- **The Rust toolchain**, to build and install thirdshift.

Running the test suite (`cargo test`) also needs **`python3`** on `PATH`: the integration tests swap in fake `gh` and `claude`, which are Python scripts in `tests/fakes/`.

### Auto mode

Sessions run headless in Claude Code's auto mode (`claude -p --permission-mode auto`), with the full permissions of the Linux user, including `sudo` if the user has it. Nobody is there to approve anything; instead, auto mode's classifier checks each action and may block ones it judges risky, such as destructive commands or actions outside the task. A blocked action the agent can't work around can end the Run as a Failed run.

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

Two more commands print to stdout and exit `0`:

```sh
thirdshift help      # every form of the command, each with a one-line description
thirdshift version   # thirdshift <version>
```

Uncommitted changes in your clone are fine: the Run works in its own worktree from `origin`, so they are simply left out. Unpushed commits on the Base branch are not: push them first, or the Run stops.

### Logs

Each session's full transcript, as Claude Code's `stream-json` output, is written to its own file under `~/.thirdshift/logs/` (created if missing):

```
~/.thirdshift/logs/<owner>-<repo>-issue-<n>-<timestamp>-implement.jsonl
~/.thirdshift/logs/<owner>-<repo>-issue-<n>-<timestamp>-repair-<i>.jsonl
```

All sessions in a Run share the Run's UTC timestamp, so a Run's logs sort together. When a Run fails, stderr ends with the path of its most recent session log, the place to start looking.

## Continuation

Running thirdshift again on an issue picks up where the last Run stopped ([ADR-0002](docs/adr/0002-existing-issue-branch-means-continue.md)). It looks at the highest-numbered Issue branch:

- **No Issue branch yet**: a fresh Run creates `issue-<n>` from the Base branch.
- **The branch exists with no pull request, or an open one**: the Run is a **Continuation**. It checks out that branch, and the agent builds on its commits instead of starting over, then creates the pull request or updates the open one. With an open pull request, that pull request's base is the Base branch, whatever you have checked out.
- **The branch's pull request was merged or closed**: finished work is never reopened. The Run starts fresh on the next number, `issue-<n>-branch-2`, `-3`, and so on. A number counts as used even if GitHub deleted the branch after merging.

So you can retry a Failed run with the same command, or start an issue on one server and continue it on another.

A Run is not idempotent: re-running builds on whatever is already on the branch, including a Failed run's work-in-progress commit.

## Failed runs

A **Failed run** is one that ends, including by Ctrl-C or a closed terminal, without an open pull request from its Issue branch that targets the Base branch, is mergeable and has passing CI. Causes include the session exiting non-zero, no pull request or one with the wrong base, and running out of Repairs.

A Failed run:

1. Commits any uncommitted work as `thirdshift: failed run (<reason>)`, with a timestamp and the hostname, and pushes the Issue branch, so nothing is lost. If the branch has no changes against the Base branch, nothing is pushed.
2. Converts its open pull request, if any, back to a draft, so a pull request only claims to be ready when the factory stands behind it. The next successful Continuation marks it ready again.
3. Cleans up as usual, prints the reason to stderr and exits non-zero.

Merges, never rebases or force-pushes: a branch worked on from several servers never loses history.

## Credits and license

thirdshift is released under the [MIT License](LICENSE).

The Factory skills in `skills/` are adapted from Matt Pocock's [mattpocock/skills](https://github.com/mattpocock/skills), changed to run headless with no human in the loop. They are used under the MIT License; its copyright notice is kept in [`skills/LICENSE`](skills/LICENSE) and is embedded in the binary and written out with the skills on every Run. The `pr` skill also reproduces part of Dex Horthy's `show-me` skill; see [`skills/pr/CREDITS.md`](skills/pr/CREDITS.md).
