# Development

## Shared documentation

The reference in [`docs/guide/`](README.md) is written once as Markdown and
published at [thirdshift.app/docs](https://thirdshift.app/docs/) as static HTML.
Edit these Markdown files, then regenerate and check the committed pages:

```sh
cd site
npm ci
npm run docs
npm test
```

`site/build-docs.mjs` owns the rendering and repository-to-site link mapping;
`site/docs/docs.css` styles the output. Commit the Markdown and generated HTML
together. `npm run docs:check` fails on stale output and runs as part of the site
tests in CI. Deployment continues to copy static files, with no server build.

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

Editing a skill in `skills/` has no effect until you rebuild and reinstall ([ADR-0001](../../docs/adr/0001-rust-binary-with-embedded-skills.md)).

The [Prompts and skills page](https://thirdshift.app/prompts/) is generated from the prompts, the `claude` and `codex` arguments and the skills, and each Session prompt is also generated as a Markdown file in [`prompts/`](../../prompts/), with a [`README.md`](../../prompts/README.md) that lists them with the `claude` and `codex` command lines. `cargo test` fails until they are regenerated after a change to any of them. Regenerate the page and `prompts/` with `UPDATE_PROMPTS=1 cargo test prompts_page`, which also deletes any file in `prompts/` that it doesn't generate.

The integration tests swap in a fake `gh`, `claude`, `codex`, `agy`, `grok`, `muse` and `opencode`, from [`tests/fakes.rs`](../../tests/fakes.rs). The first test that needs them builds them with `rustc` (the one on `PATH`, or `$RUSTC`), so the suite needs nothing else on `PATH` beyond `git` and `bash`.

### The Rust version

[`rust-toolchain.toml`](../../rust-toolchain.toml) pins the exact Rust version this repository builds with, and the `rustfmt` and `clippy` components CI runs. `rustup` reads it, so `cargo` in a checkout, CI, the release builds and the crates.io publish all use that version, and `rustup` installs it the first time it is needed. No workflow names a version of its own. The file is not in the crates.io package, so `cargo install thirdshift` builds with whatever Rust the machine has.

A newer Rust arrives as a pull request that changes `channel` in that file, so it runs through CI before `main` builds with it. Dependabot opens that pull request within a week of a stable release ([`.github/dependabot.yml`](../../.github/dependabot.yml)). To move the pin by hand, set `channel` to the new version in full, patch number included, run the three checks CI runs, and open a pull request with the change and whatever the new compiler's lints asked for:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```
## Releasing

From a clone of this repo, signed in to `gh` and the User config's default Harness, run the release script with the new version:

```sh
scripts/release.sh 0.4.0
```

It works from `origin/main` in a temporary worktree, so your checked-out branch and uncommitted changes don't matter and aren't touched. It opens a pull request from `release-<version>` that bumps the version in `Cargo.toml` and `Cargo.lock` and nothing else. Its body is a summary of the release, then the version diff. The User config’s default Harness, with its configured Model and Effort (Claude Code when no default is set), writes the summary from the version diff and the title, number and body of each pull request merged since the last tag, following the prompt in [`scripts/release-summary.md`](../../scripts/release-summary.md). The script uses `cargo run` in this checkout to share thirdshift’s config parser and Harness adapters, and runs the summary in a temporary directory without Factory skills. Because pull request titles and bodies are contributor-editable, no Harness gets its session permission bypass, and each runs with as few tools as its CLI allows. Claude runs with no tools. Codex runs read-only and sandboxed, with its shell, MCP, web, app, plugin, browser and hook tools off. Grok gets only its to-do tool. OpenCode refuses every tool call. Muse has no shell, writes or web, and one model step. agy denies every tool that needs a permission, though it can still read files. A summary that tries a refused tool usually comes back empty, so the script falls back as below. If the config is invalid, the chosen Harness fails, or it returns no summary, the script warns and uses GitHub's generated notes as the summary instead, with a note saying so. It writes the summary before it pushes anything. With `--review` (`scripts/release.sh --review 0.4.0`), it then prints the summary and asks `[y]es / [e]dit / [n]o`: `y` carries on, `e` opens the summary in `$EDITOR` and carries on with what you save, and `n` stops with no branch, pull request or tag pushed. Without `--review` it never prompts. It waits for the pull request's checks, merges it with a merge commit, then tags that merge commit `v<version>` and pushes the tag. Pushing the tag starts the release workflow (below), and the script then waits for that workflow's run on the merge commit, printing the run's URL so you can watch it. CI's run on `main` for the same commit isn't waited for. Once the release workflow run succeeds, the GitHub Release is published with its final body and the crate is on crates.io, and the script prints the GitHub Release's URL and exits `0`. It prints a line on stderr for each step. If the checks fail, it stops before merging and leaves the pull request open. If the release workflow run fails, it exits `1`, naming the failed jobs and the run's URL, and leaves the tag in place: the tag is already public, so rerun the failed jobs with `gh run rerun --failed <run id>` rather than cutting a new version. Neither wait has an overall timeout.

Before it pushes anything, it refuses a release that can't be cut and says why: a version that isn't plain `X.Y.Z` (no leading `v`), a version that isn't higher than the one in `Cargo.toml` on `origin/main`, a `v<version>` tag that already exists locally or on origin, or a latest CI run on `origin/main` that failed or hasn't finished.

If it's interrupted, run it again with the same version: it picks up the existing branch or pull request and carries on from the first step not yet done, without the checks above, which the first run passed. With `--review`, it asks about a new summary before opening a pull request for a branch already pushed. Once the tag is on the merge of the pull request, it says the release is already tagged and picks up from the release workflow run: it waits for a run still going, prints the GitHub Release's URL and exits `0` for one that succeeded, and exits `1` as above for one that failed, reporting the run's latest attempt, so a rerun of its failed jobs counts. So you can stop it with Ctrl-C while it waits and run it again to carry on waiting. A `v<version>` tag anywhere else on origin is still refused.

The tag starts the release workflow, generated by [`dist`](https://github.com/axodotdev/cargo-dist) from `dist-workspace.toml`, with the event policy applied by `python3 scripts/generate-release.py` (`--check` verifies freshness). It builds the Linux and macOS binaries and the installer, checks that the Linux binary starts on Rocky Linux 9 and in WSL2 with Ubuntu 24.04, and only then publishes the GitHub Release. It then publishes the same version to crates.io, skipping it if it is already there, and finally replaces the Release's body with the bump pull request's summary, then notes generated from the merged pull requests, then the install instructions. [`scripts/release-notes.sh`](../../scripts/release-notes.sh) builds that body; for a tag pushed by hand, with no bump pull request summary, it is the generated notes and the install instructions alone.
## Credits and license

thirdshift is released under the [MIT License](../../LICENSE).

Most Factory skills in `skills/` are adapted from Matt Pocock's [mattpocock/skills](https://github.com/mattpocock/skills), changed to run headless with no human in the loop. They are used under the MIT License; its copyright notice is kept in [`skills/LICENSE`](../../skills/LICENSE) and is embedded in the binary and written out with the skills on every Run. The `thirdshift-pr` skill also reproduces part of Dex Horthy's `show-me` skill; see [`skills/thirdshift-pr/CREDITS.md`](../../skills/thirdshift-pr/CREDITS.md).

The `thirdshift-security-audit` Factory skill is copied from Cloudflare's [security-audit-skill](https://github.com/cloudflare/security-audit-skill) at commit `c1c8a8c`, with only its frontmatter name changed. It is used under the MIT License; Cloudflare's copyright notice and licence are kept in [`skills/thirdshift-security-audit/LICENSE`](../../skills/thirdshift-security-audit/LICENSE), with the upstream repository and full commit in [`CREDITS.md`](../../skills/thirdshift-security-audit/CREDITS.md). Both are embedded and written out with the skill.
