# Installation

## Install

```sh
curl -LsSf https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh
```

The shorter `curl -fsSL https://thirdshift.app/install.sh | sh`, shown on [thirdshift.app](https://thirdshift.app), redirects to the same installer. The shell installer and the binary both come from this repository's [GitHub Releases](https://github.com/JacobStephens2/thirdshift/releases). It puts `thirdshift` in `~/.local/bin`, where Claude Code's installer puts `claude`, and needs no Rust toolchain. thirdshift is a single binary with the Factory skills compiled in, so it runs without a checkout of this repository ([ADR-0001](../../docs/adr/0001-rust-binary-with-embedded-skills.md), [ADR-0003](../../docs/adr/0003-static-binaries-through-github-releases.md)).

The installer ends by pointing you to Setup. Run `thirdshift setup` to choose your defaults, or just start a Run and thirdshift will offer it (see [User config](configuration.md#user-config)).

If you use Rust, you can install it from crates.io instead:

```sh
cargo install thirdshift
```

### Updating

```sh
thirdshift update
```

It replaces the installed binary with the latest stable GitHub Release, or says it is already on it. Messages go to stderr and stdout stays empty. It exits `0` when it updated or was already up to date, and `1` on any failure, such as no network. Updating is safe while a Run or a Spec run is using the old binary: it keeps running it. On Linux so do the child Runs it starts afterwards, a Ticket's Run or a Base fix, which it starts from the binary it is running rather than from the install path, so they run its version even once that path holds the new one or nothing at all. On macOS a child Run is started from the install path, so one started after the update runs the new binary. A Run never checks for updates or updates itself.

`thirdshift update` only replaces a copy put in place by the shell installer (the `curl` command above), which leaves an install receipt in `~/.config/thirdshift/`. It refuses to touch any other copy and lists the command that updates each other kind of install: `cargo install thirdshift` for a crates.io install, pulling and reinstalling for a build from source (see [Building from source](development.md#building-from-source)), and the `curl` command for a copy placed by hand.

Keep a single copy: with copies in more than one directory on your `PATH` (say `~/.local/bin` and `~/.cargo/bin`), you can end up running a stale one without noticing. `type -a thirdshift` lists every copy on your `PATH`.
## Supported platforms

- **Rocky Linux** 9.8 and later, x86_64.
- **Ubuntu** 24.04.3 and later, x86_64.
- **WSL2**, latest release, with Ubuntu 24.04 or later, x86_64. Keep your repositories on the Linux filesystem (e.g. `~/repos`), not under `/mnt/c`, where git is slow and file permissions misbehave. WSL1 is not supported.
- **macOS** Tahoe 26.5.2 and later, Apple Silicon.

One statically linked Linux binary covers the first three, so it doesn't depend on the host's glibc; macOS gets a native build.
## Prerequisites

The user account that runs thirdshift needs:

- The selected **Harness** installed and authenticated: **`claude`** (Claude Code, the default), **`codex`** (Codex CLI), **`agy`** (Google Antigravity CLI), **`grok`** (Grok Build), **`muse`** (Muse Code), or **`opencode`** (OpenCode, with a configured Model provider). See [Harness, Model and Effort](configuration.md#harness-model-and-effort) for each CLI's flags and quirks.
- **`gh`** (GitHub CLI), logged in.
- **`git`** with a global `user.name` and `user.email`, and credentials that can push to the repository (`gh auth setup-git` makes git use `gh`'s login). The agents commit as this identity; without it, an agent may borrow the author of the last commit.
- **`node`** (Node.js) on `PATH`, for a [Security run](passes.md#security-runs) only: the Security audit's report validators run on it.

### Unattended sessions

Sessions run headless with the full permissions of that user account, including `sudo` if the user has it, and nobody is there to approve anything. On Claude Code they run in auto mode (`claude -p --permission-mode auto`): its classifier checks each action and may block ones it judges risky, such as destructive commands or actions outside the task. A blocked action the agent can't work around can end the Run as a Failed run. The other Harnesses run with approvals and their sandbox turned off ([ADR-0013](../../docs/adr/0013-every-harness-runs-unattended-and-fully-trusted.md)), so no approval step or sandbox stands between an action and the account.

`claude -p` exits as soon as the agent ends its turn, killing any background task it started. So every prompt tells the agent to run long commands, such as tests, in the foreground, and to stop any background task it no longer needs, with the `TaskStop` tool, before it ends its turn. If a session still ends with background work running, thirdshift takes it as work the session was waiting on and gives it a **Resume**: it continues that same session once (`claude -p --resume`), asking the agent to re-run the work in the foreground and finish. If the Resume ends the same way, the work may have been abandoned, such as a hung test run the agent could not stop: thirdshift says which task was killed and carries on, so the pull request check, CI and the rest decide how the Run ends. If the Run then fails, its cause names the killed task ahead of what failed.

### What sessions leave behind

Cleanup removes only thirdshift's own worktree, local Issue branch and the temporary directory it wrote the Factory skills out to. The `.git/info/exclude` entry that keeps the linked skills out of git, `/.claude/skills/thirdshift-*`, or `/.agents/skills/thirdshift-*` on the other Harnesses, is added once and left in place, as every worktree of the repository shares the file. Anything else an agent does as your user stays. For example, one Run downloaded a JDK to `~/.local/jdk/` because the server had no Java, installed Playwright in `/tmp/pw`, and left a pull request body draft in `/tmp/`. This is by design, but worth knowing:

- **Install the target repository's toolchains up front** (language runtimes, package managers, test tools), so the agents don't download their own on every Run.
- **For UI work, install the headless browser dependencies**, e.g. `libgbm1` on Ubuntu server, which headless Chromium needs. Without it, agents can't take screenshots to check a UI change and fall back to reading the code.

### Usage and cost

When `claude` is logged in with a claude.ai subscription, Runs use the plan's usage allowance, not API billing. The `total_cost_usd` in the session logs is the API-price equivalent, not a charge. An unattended Run that hits the plan's usage limit stalls or fails partway through.
