# Rust single binary with the factory skills embedded

The factory is a Rust binary that compiles `skills/` into itself (`include_dir!`). At runtime it writes the skills and a generated `.claude-plugin/plugin.json` to a temp directory and loads them with `claude --plugin-dir`. We chose this over a TypeScript script that reads the skills from this repo on disk, so the binary is self-contained: it runs on any machine with `claude`, `gh` and `git`, and no checkout of this repo is needed.

## Considered Options

- **A TypeScript script that reads `skills/` from this repo on disk**. Rejected: every machine would need a checkout and a Node runtime.
- **Go** (amended 2026-10-28). Considered once thirdshift had grown into a process orchestrator: most of its work is starting `claude`, `gh` and `git` and waiting on them. `//go:embed` and `CGO_ENABLED=0` give the same self-contained static binary ([ADR 0003](0003-static-binaries-through-github-releases.md)), and Go would handle subprocesses, signals and concurrency with less code, no `unsafe`, and much faster builds. Not chosen: thirdshift's Run, Claim and CI logic runs unattended, and it relies on enums that carry data and on exhaustive `match`, so a new state doesn't compile until every match handles it (#1591). Go has neither. Pickup's reservation protocol ([ADR 0017](0017-pickup-reserves-issues-on-origin.md)) is model-checked with `stateright` inside `cargo test` (`tests/pickup_model.rs`), and Go has no mature equivalent. Rewriting the existing code would also cost far more than the build time it saves. Build cost is addressed within Rust instead (#174, #203, #733, #1592).

## Consequences

- Editing a skill has no effect until the binary is rebuilt and reinstalled (`cargo install --path .`).
- ~~The skills are loaded as a plugin, so they are namespaced (`/thirdshift:implement`), and their references to each other must use that namespace.~~ Replaced by [ADR 0012](0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md): the skills are linked into each worktree, named `thirdshift-<skill>`, and name each other in plain prose.
