# thirdshift

A factory that turns a GitHub issue into a ready-for-review pull request, or on request a merged one, by running unattended agent sessions against it. The humans are the day shift; the agents work the third shift, overnight, on any server, while you do something else.

Quality over quantity: a ready-for-review pull request is one the factory stands behind. When it can't, the Run fails, preserves the work and normally leaves the pull request as a draft.

## Get started

Install [GitHub CLI](https://cli.github.com/), authenticate it and your chosen Harness (Claude Code by default), and configure Git's commit identity and push access. Install the target repository's build and test tools too. See [installation and prerequisites](docs/guide/installation.md), including [supported platforms](docs/guide/installation.md#supported-platforms) and [unattended session permissions](docs/guide/installation.md#unattended-sessions).

```sh
curl -LsSf https://github.com/JacobStephens2/thirdshift/releases/latest/download/thirdshift-installer.sh | sh
thirdshift setup
cd ~/repos/widgets
thirdshift --no-merge https://github.com/acme/widgets/issues/7
```

Start from a clone of the issue's repository, on the branch the pull request should target. `--no-merge` leaves it for review even if your machine defaults to merging. Use `thirdshift merge <Issue URL>` to request a merge after review and green CI. Update the binary with `thirdshift update`.

## Documentation

Read the same guides as [Markdown in this repository](docs/guide/README.md) or on [thirdshift.app/docs](https://thirdshift.app/docs/). For a visual walkthrough, see [How thirdshift works](https://thirdshift.app/how-it-works/).

| Guide | What you'll find |
| --- | --- |
| [Installation](docs/guide/installation.md) | Install, update, supported platforms, prerequisites and session permissions |
| [Commands](docs/guide/commands.md) | Start a Run or Merge run, flags, output and exit codes |
| [Configuration](docs/guide/configuration.md) | Setup, User config, Harness, Model, Effort and email |
| [Runs and delivery](docs/guide/runs.md) | Worktrees, Claims, review, CI, Base fixes, Continuation, Failed runs and Spec runs |
| [Observing Commands](docs/guide/observing.md) | Machine status, show, logs and stopping work |
| [Scheduled work](docs/guide/passes.md) | Architect, Security and Pickup runs, Weeding, Fencing and scheduling |
| [Development](docs/guide/development.md) | Build, test, regenerate documentation and release |

The [domain glossary](CONTEXT.md) defines the project's terms. Inspect the [Session prompts](prompts/README.md) and [Factory skills](skills/), also readable on the site's [Prompts and skills page](https://thirdshift.app/prompts/). Known gaps and planned work live in the [open issues](https://github.com/JacobStephens2/thirdshift/issues).

## Credits and license

Released under the [MIT License](LICENSE). Factory skills include work adapted from [Matt Pocock](https://github.com/mattpocock/skills), [Dex Horthy](https://github.com/humanlayer/skills) and [Cloudflare](https://github.com/cloudflare/security-audit-skill); see [credits and license details](docs/guide/development.md#credits-and-license).
