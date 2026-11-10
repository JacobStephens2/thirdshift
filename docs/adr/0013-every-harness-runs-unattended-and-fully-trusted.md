# Every Harness runs unattended and fully trusted in the worktree

Sessions run with no human to answer an approval prompt or a trust question. Their commits and pushes write to the main repository's shared `.git`, outside the worktree. So every **Harness** runs its sessions, and every Resume of one, with approvals off, its sandbox off, and the worktree trusted. Each Harness carries the flags and environment that do this. They are fixed, and no user setting changes them:

| Harness | Flags and environment |
|---|---|
| Claude Code | `--permission-mode auto` |
| Codex | `--dangerously-bypass-approvals-and-sandbox` |
| agy | `--dangerously-skip-permissions`, never `--sandbox` |
| Grok Build | `--always-approve`, `GROK_FOLDER_TRUST=0` |
| Muse Code | `--yolo` |
| OpenCode | `--auto`, `--standalone` |
| Kimi Code | `-p`, whose print mode approves every action (`--yolo` and `--auto` are refused beside it), `KIMI_CODE_TRUST_WORKSPACE=1` |

The worktree is the only boundary. This generalises the Codex part of ADR 0012. Sources: `docs/research/harness-candidates.md`, `docs/research/codex-headless-harness.md`.

Exception (#1711): a Triage run with `--read-machine`, or in a repository the User config's `triage.read_machine` names, wraps the ordinary Harness invocation in a Bubblewrap filesystem sandbox. It can read the host filesystem, with only its throwaway worktree writable. Unix-domain sockets are refused, so a host service cannot write for the session. TCP stays available. A sandbox startup failure fails the Command; there is no unsandboxed fallback. Without either, Triage keeps its ordinary launch and worktree-only prompt. Harness approval and trust flags stay fixed.

## Considered Options

- **Each CLI's own sandbox.** Rejected. On stock Ubuntu (`kernel.apparmor_restrict_unprivileged_userns = 1`), bubblewrap can't start, so every sandboxed command fails under Codex and Muse. The sandboxes of agy and Grok make the main checkout's `.git` read-only, which blocks commits. A sandboxed mode for machines where it works would be a separate issue.
- **A one-time `grok --trust` in each repository's main checkout**, done or documented by Setup. Rejected. A repository nobody had trusted would run its sessions without `AGENTS.md` or the Factory skills, and nothing would fail. A scheduled Pickup run is exactly the case where nobody would notice.
- **Using the CLI's settings for approval and trust**, such as a user config that already sets always-approve. Rejected. A session would then behave differently on every machine. Passing the flags each time makes them explicit.
- **OpenCode through the user's shared background service** (its default). Rejected. A SIGTERM leaves the turn and its shell command running in the service, and thirdshift doesn't control that service's version or environment. `--standalone` keeps the server in the session's process group, and costs 2–3 s per session.

## Consequences

- A Harness's self-update is turned off in each session's environment too (`MUSE_NO_AUTO_UPDATE=1`, `GROK_DISABLE_AUTOUPDATER=1`, `AGY_CLI_DISABLE_AUTO_UPDATE=true`, `OPENCODE_DISABLE_AUTOUPDATE=1`, `KIMI_CODE_NO_AUTO_UPDATE=1`). An update in the middle of a Run could replace the binary a Resume needs. agy ignores `=1`; only `=true` works.
- Each Harness reads its own instruction file and falls back to the other. thirdshift adds the fallback where a CLI lacks one: a `GEMINI.md` link for agy, and an `AGENTS.md` link for OpenCode, both to `CLAUDE.md` at the worktree root and kept out of git. Grok reads both files when both exist, and thirdshift leaves that as it is.
- Instructions in the user's `~/.claude/` reach only the Harnesses that read them: Claude, Grok and Muse. They don't reach Codex, agy, OpenCode or Kimi Code.
