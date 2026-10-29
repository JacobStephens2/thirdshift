# thirdshift

A factory that turns GitHub issues into ready-for-review pull requests with unattended coding agents. The humans are the day shift; the agents work the third shift while you do something else.

Use **Claude Code, Codex, Antigravity CLI, Grok Build, Muse Code or OpenCode**, logged in as you. Runs use the selected agent's plan or account; thirdshift itself is free and open source.

Each Ticket run implements the issue, reviews the change against the issue and your repository's standards, and keeps the pull request mergeable and passing CI. It leaves the pull request for your review, or merges it when asked. If a run fails, it reports what work was preserved and how to continue.

Read the **[user manual](docs/guide.md)** here on GitHub or **[on thirdshift.app](https://thirdshift.app/docs/)**. For a visual introduction, see [How thirdshift works](https://thirdshift.app/how-it-works/).

## Install

```sh
curl -LsSf https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh
thirdshift setup
```

The installer puts a single binary in `~/.local/bin`; no Rust toolchain or checkout of this repo is needed. Supported: x86_64 Linux (Rocky, Ubuntu and WSL2) and Apple Silicon macOS. See [supported versions](docs/guide.md#supported-platforms) and [other installation methods](docs/guide.md#install).

Have `git`, authenticated `gh`, and one of the supported coding agent CLIs installed. Set git's `user.name` and `user.email` and make sure it can push. [Prerequisites and permissions](docs/guide.md#prerequisites) covers setup and access: sessions run unattended as your user, with access to that account's files, credentials and tools.

Use `thirdshift update` to update a shell-installer copy.

## Usage

From a clone of the issue's repository, on the branch you want the pull request to target:

```sh
cd ~/repos/widgets
thirdshift https://github.com/acme/widgets/issues/7
```

You can also use the issue number from that clone, choose a Harness, or ask for a merge:

```sh
thirdshift 7                # leave the pull request ready for review
thirdshift 7 harness codex   # use Codex instead of the default Claude Code
thirdshift merge 7           # also merge the pull request when it is ready
```

These examples use the default settings. Command options override your [User config](docs/guide.md#user-config); `--no-merge` leaves a pull request for review even if your defaults request merging.

The Ticket run works in its own git worktree. Your checkout's uncommitted changes stay out of it; push any Base branch commits first. thirdshift watches CI and starts Repair sessions for merge conflicts or failing checks. Run the same issue again to [continue existing work](docs/guide.md#continuation).

Use `thirdshift help` for the command reference, or read [Ticket run behavior](docs/guide.md#what-a-ticket-run-does), [command options](docs/guide.md#usage) and [failure recovery](docs/guide.md#failed-ticket-runs).

## Workflows

| Start with | Command | What happens |
| --- | --- | --- |
| One GitHub issue | `thirdshift 7` | A [Ticket run](docs/guide.md#usage) delivers one pull request, with optional Self-merge. |
| An issue with sub-issues | `thirdshift 42` | A [Spec run](docs/guide.md#spec-runs) works through Tickets in dependency order, in parallel where possible, and delivers one Spec PR. |
| Issues marked `ready-for-agent` | `thirdshift pickup` | A [Pickup run](docs/guide.md#pickup-runs) takes the lowest-numbered Ready issue. |
| Architecture improvements | `thirdshift architect` | An [Architect run](docs/guide.md#architect-runs) reviews the codebase, publishes a plan and builds it, or files an idea for triage. |
| A security audit | `thirdshift secure` | A [Security run](docs/guide.md#security-runs) records findings privately; fixing a reproduced finding is optional. |
| Merged factory work | `thirdshift score` | A [Scoring run](docs/guide.md#scoring-runs) samples and grades it, and proposes Bug origins; `thirdshift scores --json` lists samples, five criterion Scores, calibration and proposals by week; `thirdshift scores <pull request> --mine` records yours before revealing the model's. |
| One skill | `thirdshift skill <skill> [issue <issue>]` | A [standalone skill](docs/guide.md#standalone-skills) runs headlessly and prints its final message. |
| A Custom process | `thirdshift process <name> [issue <issue>]` | An experimental [Custom process](docs/guide.md#custom-processes) runs skill, command or catalogue Steps from `~/.thirdshift/processes/<name>.toml`, in named throwaway worktrees when the file declares them. The catalogue Step `run-published-issue` starts a Ticket run or a Spec run on the issue the session Step before it published. An optional Step can fail without stopping the process. `issue <issue>` gives every session Step that issue. One Issue-branch worktree plus a `delivery` Step publishes a ready-for-review pull request from that Issue branch and then removes the Claim. A `self-merge` Step directly after it merges when `merge` or the User config asks, and a file can only tighten limits. With no issue, and no Issue-branch worktree, it is a Pass and skips while the repository is at its Command limit. Status, the Console and the Activity log list it as Process `<name>`. A failed one is in Needs you, and it sends a Run notification under the same email rules as a Ticket run. A skipped Process sends none. Each Step may name its Harness, Model and Effort, otherwise inheriting the Command's. |

A Custom process can use `implement` → `delivery` on an Issue-branch worktree for the Ticket run's implementation and bug diagnosis. An unreproduced bug ends successfully with `needs-info`, skipping later Steps and cleaning up the Claim and worktree.

Each Architect, Pickup or Security run makes one pass. Use cron or another operating system scheduler to repeat it. See [scheduling](docs/guide.md#on-a-schedule), [Architect runs every few minutes](docs/guide.md#architect-runs-every-few-minutes) and [Security runs on a schedule](docs/guide.md#security-runs-on-a-schedule-with-fixing-allowed).

## Monitor and manage work

From another terminal, inspect this machine's work across repositories:

```sh
thirdshift status
thirdshift show 'widgets#7'
thirdshift logs 'widgets#7' --follow
```

`status` groups Commands by what needs you, what is ready for review, what is landing and what is still working. `show` includes stages and child Ticket runs. [Session traces](docs/guide.md#logs) retain agent events and timed work; you can inspect the span tree or export a Command's whole Trace.

For a live browser view of the same Inbox, with stages, logs and Session traces:

```sh
thirdshift console --no-sign-in
```

Open the localhost address it prints. The Console never starts work. Use [`thirdshift stop <command>`](docs/guide.md#stopping-a-command) to stop a live Command and wait for cleanup; add `--requeue` to return its issue to Pickup. A signed-in Console account can do the same.

### Console on a server

The Console and `status` show the serving OS account's Commands by default. To add readable Commands from other local accounts, use `thirdshift console other-accounts readable` (or a comma-separated list of account names). Other-account Commands show their owner and are read-only. See [account visibility](docs/guide.md#other-local-accounts) for filesystem access requirements.

<a name="nginx"></a>

The Console can run as a persistent service behind HTTPS and sign-in. See [server installation](docs/guide.md#console-on-a-server) for configuration, passwords, the generated Linux installer and Caddy, Apache or [nginx](docs/guide.md#nginx) setup.

## Documentation

- [Configuration](docs/guide.md#user-config), [Harness, Model and Effort](docs/guide.md#harness-model-and-effort), and [email notifications](docs/guide.md#email).
- [Machine status](docs/guide.md#machine-status), [showing Commands and reading logs](docs/guide.md#show-and-read-one-command), and [stopping a Command](docs/guide.md#stopping-a-command).
- [Console installation](docs/guide.md#console-installation) and [Console Themes and browser tests](console/README.md).
- [Host resources](docs/guide.md#host-resources): `thirdshift resources --json` and the Console's Resources page show RAM, CPU load and configurable limits that delay new sessions.
- [Building and testing](docs/guide.md#building-from-source) and [releasing](docs/guide.md#releasing).
- [Exact prompts](prompts/README.md) and [Factory skills](skills/), also [readable on the site](https://thirdshift.app/prompts/).
- [Built-in processes](processes/) are compiled into the binary. `thirdshift process list` lists Processes and marks Built-in processes; `thirdshift process show <name>` prints a Process file exactly so you can copy it under a new Custom process name.
- [Domain vocabulary](GLOSSARY.md) and [architectural decisions](docs/adr/).
- [Open issues](https://github.com/JacobStephens2/thirdshift/issues) for known gaps and planned work.

## Credits and license

Released under the [MIT License](LICENSE). Factory skills include work by [Matt Pocock](https://github.com/mattpocock/skills), [Dex Horthy](skills/thirdshift-pr/CREDITS.md) and [Cloudflare](skills/thirdshift-security-audit/CREDITS.md). See [full credits and license notices](docs/guide.md#credits-and-license).

<details>
<summary>Links from the former README manual</summary>

These section links keep older bookmarks useful. The complete explanations now live in the [user manual](docs/guide.md).

- <a name="what-a-run-does"></a>[What a Ticket run does](docs/guide.md#what-a-ticket-run-does)
- <a name="the-claim"></a>[The Claim](docs/guide.md#the-claim)
- <a name="when-the-claim-ends"></a>[When the Claim ends](docs/guide.md#when-the-claim-ends)
- <a name="status"></a>[Status](docs/guide.md#status)
- <a name="machine-status"></a>[Machine status](docs/guide.md#machine-status)
- <a name="show-and-read-one-command"></a>[Show and read one Command](docs/guide.md#show-and-read-one-command)
- <a name="updating"></a>[Updating](docs/guide.md#updating)
- <a name="supported-platforms"></a>[Supported platforms](docs/guide.md#supported-platforms)
- <a name="prerequisites"></a>[Prerequisites](docs/guide.md#prerequisites)
- <a name="unattended-sessions"></a>[Unattended sessions](docs/guide.md#unattended-sessions)
- <a name="what-sessions-leave-behind"></a>[What sessions leave behind](docs/guide.md#what-sessions-leave-behind)
- <a name="usage-and-cost"></a>[Usage and cost](docs/guide.md#usage-and-cost)
- <a name="user-config"></a>[User config](docs/guide.md#user-config)
- <a name="harness-model-and-effort"></a>[Harness, Model and Effort](docs/guide.md#harness-model-and-effort)
- <a name="email"></a>[Email](docs/guide.md#email)
- <a name="run-notifications"></a>[Run notifications](docs/guide.md#run-notifications)
- <a name="stopping-a-command"></a>[Stopping a Command](docs/guide.md#stopping-a-command)
- <a name="logs"></a>[Logs](docs/guide.md#logs)
- <a name="foreign-commits-in-a-merge-run"></a>[Foreign commits in a Ticket run that Self-merges](docs/guide.md#foreign-commits-in-a-ticket-run-that-self-merges)
- <a name="base-fix"></a>[Base fix](docs/guide.md#base-fix)
- <a name="one-base-fix-per-base-branch"></a>[One Base fix per Base branch](docs/guide.md#one-base-fix-per-base-branch)
- <a name="continuation"></a>[Continuation](docs/guide.md#continuation)
- <a name="failed-runs"></a>[Failed ticket runs](docs/guide.md#failed-ticket-runs)
- <a name="an-inherited-failure-links-the-base-branchs-checks"></a>[An Inherited failure links the Base branch's checks](docs/guide.md#an-inherited-failure-links-the-base-branchs-checks)
- <a name="spec-runs"></a>[Spec runs](docs/guide.md#spec-runs)
- <a name="architect-runs"></a>[Architect runs](docs/guide.md#architect-runs)
- <a name="one-run-notification"></a>[One Run notification](docs/guide.md#one-run-notification)
- <a name="no-strong-candidate"></a>[No Strong candidate](docs/guide.md#no-strong-candidate)
- <a name="an-architect-idea-waits-for-triage"></a>[An Architect idea waits for triage](docs/guide.md#an-architect-idea-waits-for-triage)
- <a name="a-ready-issue-goes-first"></a>[A Ready issue goes first](docs/guide.md#a-ready-issue-goes-first)
- <a name="one-at-a-time"></a>[The Command limit](docs/guide.md#the-command-limit)
- <a name="one-architect-plan-at-a-time"></a>[One Architect plan at a time](docs/guide.md#one-architect-plan-at-a-time)
- <a name="on-a-schedule"></a>[On a schedule](docs/guide.md#on-a-schedule)
- <a name="weeding"></a>[Architect runs every few minutes](docs/guide.md#architect-runs-every-few-minutes)
- <a name="security-runs"></a>[Security runs](docs/guide.md#security-runs)
- <a name="security-review"></a>[Security review](docs/guide.md#security-review)
- <a name="fencing"></a>[Security runs on a schedule, with fixing allowed](docs/guide.md#security-runs-on-a-schedule-with-fixing-allowed)
- <a name="pickup-runs"></a>[Pickup runs](docs/guide.md#pickup-runs)
- <a name="the-claim-limit"></a>[The Failure limit](docs/guide.md#the-failure-limit)
- <a name="the-sweep"></a>[The Sweep](docs/guide.md#the-sweep)
- <a name="why-an-issue-was-passed-over"></a>[Why an issue was passed over](docs/guide.md#why-an-issue-was-passed-over)
- <a name="one-run-notification-for-the-issue-taken"></a>[One Run notification for the issue taken](docs/guide.md#one-run-notification-for-the-issue-taken)
- <a name="a-pickup-run-on-a-schedule"></a>[A Pickup run on a schedule](docs/guide.md#a-pickup-run-on-a-schedule)
- <a name="building-from-source"></a>[Building from source](docs/guide.md#building-from-source)
- <a name="the-rust-version"></a>[The Rust version](docs/guide.md#the-rust-version)
- <a name="releasing"></a>[Releasing](docs/guide.md#releasing)

</details>
