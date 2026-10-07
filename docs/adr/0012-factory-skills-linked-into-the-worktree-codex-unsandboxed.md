# Factory skills are linked into the worktree under a `thirdshift-` prefix, and Codex sessions run unsandboxed

A **Command** can now run its sessions on either **Harness**: Claude Code or Codex. Codex has nothing like `claude --plugin-dir`. It finds skills only when they are installed into `$CODEX_HOME`, or placed in a repository's `.agents/skills/`. So the **Factory skills** stop being loaded as a plugin for both harnesses. thirdshift writes the embedded skills out once per Command, links each into the Run's worktree (`.claude/skills/` for Claude, `.agents/skills/` for Codex), and keeps them out of git with a `.git/info/exclude` entry. Each skill is named `thirdshift-<skill>` in `skills/` itself (directory, frontmatter `name:`, and the skills' references to each other), so it can't collide with a repository's own skills of the same name, as this repository's `.agents/skills/code-review` would. A **Session prompt** loads its skill with the harness's sigil on its first line (`/thirdshift-implement` or `$thirdshift-implement`), and names the others in plain prose. This replaces the plugin consequence of ADR 0001.

Codex sessions, and every Resume of one, run with `--dangerously-bypass-approvals-and-sandbox`. `codex exec` never asks for approval, and on a stock Ubuntu machine its bubblewrap sandbox can't start (`kernel.apparmor_restrict_unprivileged_userns = 1`), which fails every command. A worktree's commits and pushes also need to write to the main repository's shared `.git`, outside the sandbox's writable root. The worktree is the only boundary, as it is for Claude's auto mode. Sources: `docs/research/codex-headless-harness.md`.

## Considered Options

- **A temporary `CODEX_HOME` holding the skills.** Rejected: it drops the user's `~/.codex/config.toml` and global `AGENTS.md`, breaks keyring auth, and a Resume must reuse the same home.
- **Keep the Claude plugin and link skills only for Codex.** Rejected: two mechanisms and two sets of skill names (`/thirdshift:x` and `thirdshift-x`) for the same skills.
- **Plain skill names.** Rejected: they collide with a target repository's own skills.
- **Rename the skills at write time.** Rejected: the embedded skills and the Prompts and skills page would not show the names sessions actually see.
- **`--approve-for-me` (workspace-write with an automatic reviewer).** Rejected for now: on a machine where the sandbox can't start, every command escalates. A sandboxed Codex mode, for machines where bubblewrap works, is a separate issue.

## Consequences

- `.git/info/exclude` is shared by every worktree of a repository. The `thirdshift-*` patterns are added once and left there. They are harmless.
- Each Codex session adds a permanent `trust_level = "trusted"` entry for the target repository's root to the user's `~/.codex/config.toml`.
- The user's Claude-only instructions and hooks (`~/.claude/`) don't reach Codex sessions. Codex reads `AGENTS.md`, and falls back to `CLAUDE.md` when the repository has none.

Generalised to every Harness by ADR 0013.
