# thirdshift

A factory that turns a GitHub issue into a ready-for-review pull request, or on request a merged one, by running unattended agent sessions. The humans are the day shift; the agents work the third shift, overnight, while you do something else.

Quality over quantity: a pull request marked ready for review is one the factory stands behind. When it can't stand behind the work, it preserves the work and leaves a draft.

Read the **[user manual](docs/guide.md)** here on GitHub or **[on thirdshift.app](https://thirdshift.app/docs/)**. Both come from the same Markdown. For a visual introduction, see [How thirdshift works](https://thirdshift.app/how-it-works/).

## Install

```sh
curl -LsSf https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh
```

The installer puts a single binary in `~/.local/bin`; no Rust toolchain or checkout of this repo is needed. Supported: x86_64 Linux (Rocky, Ubuntu and WSL2) and Apple Silicon macOS. See [supported versions](docs/guide.md#supported-platforms) and [other installation methods](docs/guide.md#install).

Have `git` and authenticated `gh` and a Harness installed: Claude Code by default, or Codex, Antigravity CLI, Grok Build, Muse Code or OpenCode. Set git's `user.name` and `user.email` and make sure it can push. [Prerequisites and permissions](docs/guide.md#prerequisites) covers setup and access: sessions run unattended as your user, with access to that account's files, credentials and tools.

```sh
thirdshift setup     # choose defaults
thirdshift update    # update a shell-installer copy
```

## Usage

From a clone of the issue's repository, on the branch you want the pull request to target:

```sh
cd ~/repos/widgets
thirdshift https://github.com/acme/widgets/issues/7
```

The Run works in its own worktree, implements and reviews the issue, opens a pull request, and keeps it mergeable and green. It leaves the pull request ready for your review; `thirdshift merge <Issue URL>` also merges it. Your checkout's uncommitted changes stay out of the Run. Push any Base branch commits first.

Use `thirdshift help` for the command reference, or read [Run behavior](docs/guide.md#what-a-run-does), [command options](docs/guide.md#usage) and [failure recovery](docs/guide.md#failed-runs).

## Workflows

| Start with | Workflow and details |
| --- | --- |
| One GitHub issue | [Run and Merge run](docs/guide.md#usage) |
| An issue with sub-issues | [Spec run](docs/guide.md#spec-runs): works through Tickets in dependency order |
| Architecture improvements | [Architect run](docs/guide.md#architect-runs): reviews, publishes a plan and builds it |
| An issue marked ready | [Pickup run](docs/guide.md#pickup-runs): takes the lowest-numbered Ready issue |
| A security audit | [Security run](docs/guide.md#security-runs): records findings privately; can fix one reproduced finding |

The operating system schedules Architect, Pickup and Security runs. See [scheduling](docs/guide.md#on-a-schedule), [Weeding](docs/guide.md#weeding) and [Fencing](docs/guide.md#fencing).

## Documentation

- [Configuration](docs/guide.md#user-config), [Harness, Model and Effort](docs/guide.md#harness-model-and-effort), and [email notifications](docs/guide.md#email).
- [Machine status](docs/guide.md#machine-status), [showing Commands and reading logs](docs/guide.md#show-and-read-one-command), and [stopping a Command](docs/guide.md#stopping-a-command).
- [Building and testing](docs/guide.md#building-from-source) and [releasing](docs/guide.md#releasing).
- [Exact prompts](prompts/README.md) and [Factory skills](skills/), also [readable on the site](https://thirdshift.app/prompts/).
- [Domain vocabulary](CONTEXT.md) and [architectural decisions](docs/adr/).
- [Open issues](https://github.com/JacobStephens2/thirdshift/issues) for known gaps and planned work.

## Credits and license

Released under the [MIT License](LICENSE). Factory skills include work by [Matt Pocock](https://github.com/mattpocock/skills), [Dex Horthy](skills/thirdshift-pr/CREDITS.md) and [Cloudflare](skills/thirdshift-security-audit/CREDITS.md). See [full credits and license notices](docs/guide.md#credits-and-license).

<details>
<summary>Links from the former README manual</summary>

These section links keep older bookmarks useful. The complete explanations now live in the [user manual](docs/guide.md).

- <a name="what-a-run-does"></a>[What a Run does](docs/guide.md#what-a-run-does)
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
- <a name="foreign-commits-in-a-merge-run"></a>[Foreign commits in a Merge run](docs/guide.md#foreign-commits-in-a-merge-run)
- <a name="base-fix"></a>[Base fix](docs/guide.md#base-fix)
- <a name="one-base-fix-per-base-branch"></a>[One Base fix per Base branch](docs/guide.md#one-base-fix-per-base-branch)
- <a name="continuation"></a>[Continuation](docs/guide.md#continuation)
- <a name="failed-runs"></a>[Failed runs](docs/guide.md#failed-runs)
- <a name="an-inherited-failure-links-the-base-branchs-checks"></a>[An Inherited failure links the Base branch's checks](docs/guide.md#an-inherited-failure-links-the-base-branchs-checks)
- <a name="spec-runs"></a>[Spec runs](docs/guide.md#spec-runs)
- <a name="architect-runs"></a>[Architect runs](docs/guide.md#architect-runs)
- <a name="one-run-notification"></a>[One Run notification](docs/guide.md#one-run-notification)
- <a name="no-strong-candidate"></a>[No Strong candidate](docs/guide.md#no-strong-candidate)
- <a name="an-architect-idea-waits-for-triage"></a>[An Architect idea waits for triage](docs/guide.md#an-architect-idea-waits-for-triage)
- <a name="a-ready-issue-goes-first"></a>[A Ready issue goes first](docs/guide.md#a-ready-issue-goes-first)
- <a name="one-at-a-time"></a>[One at a time](docs/guide.md#one-at-a-time)
- <a name="one-architect-plan-at-a-time"></a>[One Architect plan at a time](docs/guide.md#one-architect-plan-at-a-time)
- <a name="on-a-schedule"></a>[On a schedule](docs/guide.md#on-a-schedule)
- <a name="weeding"></a>[Weeding](docs/guide.md#weeding)
- <a name="security-runs"></a>[Security runs](docs/guide.md#security-runs)
- <a name="security-review"></a>[Security review](docs/guide.md#security-review)
- <a name="fencing"></a>[Fencing](docs/guide.md#fencing)
- <a name="pickup-runs"></a>[Pickup runs](docs/guide.md#pickup-runs)
- <a name="the-claim-limit"></a>[The Claim limit](docs/guide.md#the-claim-limit)
- <a name="the-sweep"></a>[The Sweep](docs/guide.md#the-sweep)
- <a name="why-an-issue-was-passed-over"></a>[Why an issue was passed over](docs/guide.md#why-an-issue-was-passed-over)
- <a name="one-run-notification-for-the-issue-taken"></a>[One Run notification for the issue taken](docs/guide.md#one-run-notification-for-the-issue-taken)
- <a name="a-pickup-run-on-a-schedule"></a>[A Pickup run on a schedule](docs/guide.md#a-pickup-run-on-a-schedule)
- <a name="building-from-source"></a>[Building from source](docs/guide.md#building-from-source)
- <a name="the-rust-version"></a>[The Rust version](docs/guide.md#the-rust-version)
- <a name="releasing"></a>[Releasing](docs/guide.md#releasing)

</details>
