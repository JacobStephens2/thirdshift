# Running `thirdshift` on Linux on an Apple Silicon MacBook

Research date: 2026-10-30

**Question.** What is the best way to run `thirdshift` on Linux on this MacBook (macOS Tahoe 26.5.2, Apple M5 Pro, 18 cores, 64 GB RAM, `arm64`, with Docker 29.8.2 installed)?

**Method.**

- Inspected this workstation's environment (`sw_vers`, `sysctl`, `docker version`, Homebrew formulae/casks).
- Read `thirdshift`'s Linux-specific runtime, sandboxing, process-identity, scope, Console and release code: [`src/sandbox.rs`](../../src/sandbox.rs), [`src/sandbox/code_grader.rs`](../../src/sandbox/code_grader.rs), [`src/setup/sandbox.rs`](../../src/setup/sandbox.rs), [`src/scope.rs`](../../src/scope.rs), [`src/console/install.rs`](../../src/console/install.rs), [`src/console/watching.rs`](../../src/console/watching.rs), [`src/logs/process_identity.rs`](../../src/logs/process_identity.rs), [`src/skills/directory.rs`](../../src/skills/directory.rs), [`src/child_run.rs`](../../src/child_run.rs), [`src/resources.rs`](../../src/resources.rs), [`dist-workspace.toml`](../../dist-workspace.toml), [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml), [`.github/workflows/release-smoke.yml`](../../.github/workflows/release-smoke.yml), [`docs/ci-performance.md`](../ci-performance.md) and [`docs/guide.md`](../guide.md).
- Read primary documentation for [Lima](https://lima-vm.io/docs/), [OrbStack](https://docs.orbstack.dev/), [Docker Engine and Docker Desktop](https://docs.docker.com/), [Apple `Virtualization.framework`](https://developer.apple.com/documentation/virtualization), [Ubuntu 24.04 LTS Release Notes](https://discourse.ubuntu.com/t/ubuntu-24-04-lts-noble-numbat-release-notes/39890), [Tart](https://tart.run/) and [UTM](https://docs.getutm.app/).
- Every claim is cited inline to the codebase or primary vendor documentation.

**Out of scope.** Adding an `aarch64-unknown-linux-musl` release target to `dist-workspace.toml`, or modifying `thirdshift` itself.

## TL;DR

- **`thirdshift` on Linux expects a real Linux host, not an unprivileged container.** Its Evals, Trials, Graders, Security sandbox probe and Console installer rely on:
  1. **Unprivileged user, PID, IPC, UTS, network and mount namespaces** via `bwrap` (`bubblewrap`) and `socat` ([`src/sandbox.rs` L327-595](../../src/sandbox.rs#L327-L595), [`src/setup/sandbox.rs` L66-152](../../src/setup/sandbox.rs#L66-L152)).
  2. **A running `systemd` user manager (`systemd-run --user --scope`), `loginctl enable-linger`, `journalctl --user`, and writable cgroup v2 `memory.events`** to enforce and observe per-Trial and per-Grader memory caps ([`src/scope.rs` L147-456](../../src/scope.rs#L147-L456)).
  3. **Fast native filesystem metadata and `inotify`**, because `thirdshift` workflows and tests are dominated by `git` process spawning and worktree inspection ([`docs/ci-performance.md` L271-396](../ci-performance.md#L271-L396)), and the Console watches logs via Linux `inotify` and `pidfd_open` ([`src/console/watching.rs` L59-175](../../src/console/watching.rs#L59-L175)).
- **Don't use a Docker container for full `thirdshift` workflows.** Although the `docker` CLI (`29.8.2`) is installed on this Mac (its daemon is currently stopped), Docker's default seccomp and capability profile blocks `unshare`, `clone` namespace flags, `mount` and `pivot_root` ([Docker: Seccomp security profiles](https://docs.docker.com/engine/security/seccomp/)), and standard containers do not boot `systemd` as PID 1 or run a `systemd --user` manager. Making both `bwrap` and `systemd-run --user --scope` work inside Docker requires `--privileged` (or `--cap-add SYS_ADMIN --cap-add NET_ADMIN --security-opt seccomp=unconfined --security-opt apparmor=unconfined` plus a systemd init container and cgroup v2 delegation), which strips the container boundary while adding nested-namespace and cgroup friction.
- **Build `thirdshift` natively inside the `aarch64` Linux VM (`cargo install --path .`), and enable Rosetta 2 for `x86_64` compatibility.** `dist-workspace.toml` currently ships `x86_64-unknown-linux-musl` and `aarch64-apple-darwin` ([`dist-workspace.toml` L12](../../dist-workspace.toml#L12)), so `thirdshift-installer.sh` has no pre-built `aarch64` Linux binary. Building natively on `aarch64-unknown-linux-gnu` inside the VM takes under a minute on the M5 Pro and avoids binary translation during runs and tests; enabling Apple's `VZLinuxRosettaDirectoryShare` lets the same VM also run the released `x86_64-unknown-linux-musl` binary and any `x86_64`-only agent tooling at near-native speed ([Apple: `VZLinuxRosettaDirectoryShare`](https://developer.apple.com/documentation/virtualization/vzlinuxrosettadirectoryshare)).
- **Keep repositories and `~/.thirdshift` on the Linux guest's native `ext4` disk, not on a VirtioFS mount of `/Users/...`.** Just as the user manual warns against `/mnt/c` on WSL2 ([`docs/guide.md` L428](../guide.md#L428)), keeping clones under `/home/<user>/repos` inside the VM avoids macOS Spotlight (`mds`/`mdworker_shared`) indexing throwaway worktrees ([`docs/ci-performance.md` L398-401](../ci-performance.md#L398-L401)), avoids VirtioFS metadata latency across thousands of `git rev-parse` calls, and preserves native kernel `inotify` events ([Lima: Filesystem mounts](https://lima-vm.io/docs/config/mount/)).
- **Recommendation:**
  1. **Best overall (open-source, dedicated kernel, matches CI):** **Lima (`brew install lima`)** running **Ubuntu 24.04 LTS (`aarch64`)** on Apple's `Virtualization.framework` (`--vm-type=vz --rosetta`), with host home mounts disabled (`--mount-none` or a single narrow project mount) so unattended sessions cannot touch macOS files.
  2. **Best low-overhead / fast-UX alternative:** **OrbStack (`brew install --cask orbstack`)** using an **isolated machine** (`orb create --isolated ubuntu:noble thirdshift`). It starts in seconds, releases idle RAM dynamically, and auto-forwards ports and `.orb.local` domains ([OrbStack: Isolated machines](https://docs.orbstack.dev/machines/isolated)), with two caveats: all OrbStack machines and containers share a single custom Linux kernel rather than Ubuntu's stock kernel ([OrbStack: Architecture](https://docs.orbstack.dev/architecture)), and commercial use requires a Pro license at $8/user/month ([OrbStack: Licensing](https://docs.orbstack.dev/licensing)).

---

## 1. What `thirdshift` requires on Linux

Inspecting the repository shows six concrete Linux requirements beyond basic POSIX utilities (`git`, `gh`, `curl`, `bash`):

### 1.1 Bubblewrap (`bwrap`) namespaces and `socat` (`src/sandbox.rs`, `src/setup/sandbox.rs`)

- **Trial and Model Grader sandboxes** ([`src/sandbox.rs` L327-446](../../src/sandbox.rs#L327-L446), `base_argv` at [L560-595](../../src/sandbox.rs#L560-L595)) require `bwrap` and `socat` on `PATH` unless `THIRDSHIFT_TRIAL_NO_SANDBOX=1` is set ([L195-197](../../src/sandbox.rs#L195-L197)).
- Each session launches `bwrap` with:
  ```text
  --unshare-pid --unshare-ipc --unshare-uts --unshare-net
  --die-with-parent --new-session
  --dev /dev --proc /proc --tmpfs /tmp
  ```
  followed by `--ro-bind` of `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, `/etc`, `/usr/local`, `/opt`, toolchains (`~/.rustup`, `~/.cargo`, `~/.nvm`), a fresh `$HOME`, and the allowlisting CONNECT proxy's Unix domain socket at `/sandbox/proxy.sock`, which `socat` bridges to `127.0.0.1:3128` inside the isolated network namespace ([`src/sandbox.rs` L314-319](../../src/sandbox.rs#L314-L319), [L455-467](../../src/sandbox.rs#L455-L467)).
- **Code Graders** ([`src/sandbox/code_grader.rs` L119-232](../../src/sandbox/code_grader.rs#L119-L232)) also require `bwrap` with `--json-status-fd` and an offline network namespace (`--unshare-net` with no proxy relay).
- **Security sandbox probe** (`thirdshift setup`, [`src/setup/sandbox.rs` L66-152](../../src/setup/sandbox.rs#L66-L152)) runs only on `#[cfg(target_os = "linux")]` (on macOS it prints `Security sandbox probe: unavailable on macos`, [L48-63](../../src/setup/sandbox.rs#L48-L63)). On Linux it spawns `bwrap --unshare-all --die-with-parent --new-session --ro-bind / / --dev /dev --proc /proc ...` and verifies that:
  1. `bwrap` starts and completes within 3 seconds;
  2. connecting to a host `127.0.0.1` listener fails (network isolation);
  3. opening the host User config for writing fails with `EROFS` (read-only host home);
  4. writing inside the scratch mount with a dummy `HOME` succeeds.
- **Ubuntu 24.04 AppArmor restriction:** Ubuntu 24.04 LTS enables `kernel.apparmor_restrict_unprivileged_userns = 1` by default, blocking unprivileged processes from entering user namespaces (`CLONE_NEWUSER`, which `bwrap` uses whenever it is not setuid root) unless permitted by an AppArmor profile ([Ubuntu 24.04 LTS Release Notes: Security](https://discourse.ubuntu.com/t/ubuntu-24-04-lts-noble-numbat-release-notes/39890)). [`src/setup/sandbox.rs` L14-28](../../src/setup/sandbox.rs#L14-L28) documents the exact per-binary AppArmor profile remedy:
  ```sh
  sudo tee /etc/apparmor.d/bwrap <<'EOF'
  abi <abi/4.0>,
  include <tunables/global>
  profile bwrap /usr/bin/bwrap flags=(unconfined) {
    userns,
    include if exists <local/bwrap>
  }
  EOF
  sudo apparmor_parser -r /etc/apparmor.d/bwrap
  ```

### 1.2 `systemd --user` manager and cgroup v2 memory scopes (`src/scope.rs`)

- Every Trial and Grader is launched inside a transient `systemd` user scope ([`src/scope.rs` L378-456](../../src/scope.rs#L378-L456)):
  ```text
  systemd-run --user --scope --unit=thirdshift-<kind>-<pid>-<n>-<detail>.scope
    --property=MemoryHigh=1G
    --property=MemoryMax=2G
    --property=MemorySwapMax=1G
    --property=OOMPolicy=kill
    -- <thirdshift> --oom-exec <program> <args...>
  ```
- Before spawning, `scope::available()` ([`src/scope.rs` L147-155](../../src/scope.rs#L147-L155)) checks that `systemd-run` is on `PATH` and `systemctl --user show -p SystemState --value` exits 0 (inferring `XDG_RUNTIME_DIR=/run/user/<euid>` on Linux when absent, [L159-169](../../src/scope.rs#L159-L169)). Without a running user manager, `scope::launch` aborts before dispatch ([L437-442](../../src/scope.rs#L437-L442)).
- While the scope runs, `Scope::sample()` ([`src/scope.rs` L235-299](../../src/scope.rs#L235-L299), [L314-374](../../src/scope.rs#L314-L374)) queries `systemctl --user show <unit> -p ControlGroup --value`, reads `/sys/fs/cgroup/<group>/memory.events` for `oom_kill` and `oom_group_kill`, and checks `journalctl --user -u <unit>` for `Failed with result 'oom-kill'`.
- Consequently, the Linux environment **must** boot `systemd` as PID 1, mount unified cgroup v2 at `/sys/fs/cgroup` with the `memory` controller delegated to user slices, and have `loginctl enable-linger <user>` enabled so cron/background passes keep `/run/user/<uid>` alive.

### 1.3 Linux-specific syscalls, `/proc`, and `inotify`

| Subsystem | Linux mechanism used by `thirdshift` | Code reference |
| --- | --- | --- |
| Process identity & `thirdshift stop` | Reads `/proc/<pid>/stat` start-time ticks and `/proc/sys/kernel/random/boot_id`; pins PID via `SYS_pidfd_open`, rechecks start identity, and sends `SIGTERM` via `SYS_pidfd_send_signal` | [`src/logs/process_identity.rs` L5-34, L91-103](../../src/logs/process_identity.rs#L5-L103) |
| Console live process & file watching | `SYS_pidfd_open` wrapped in Tokio `AsyncFd` for zero-polling process exit detection; `notify::recommended_watcher` (`inotify` on Linux) for log file changes | [`src/console/watching.rs` L63-91, L168-176](../../src/console/watching.rs#L63-L176) |
| Symlink-safe skill directory linking | `openat` with `O_PATH \| O_DIRECTORY \| O_NOFOLLOW \| O_CLOEXEC`, `mkdirat`, `unlinkat`, `symlinkat` | [`src/skills/directory.rs` L15-137](../../src/skills/directory.rs#L15-L137) |
| Self-update resilience for child Runs | Spawns child Ticket/Base-fix runs via `/proc/self/exe` (which survives binary replacement at `~/.local/bin/thirdshift`) and sets process comm via `prctl(PR_SET_NAME)` | [`src/child_run.rs` L619-625, L702-712](../../src/child_run.rs#L619-L712) |
| Console systemd service installer | Generates `/etc/systemd/system/thirdshift-console.service` (`systemctl daemon-reload/enable/restart`, `journalctl -u`) plus optional Caddy or Apache config | [`src/console/install.rs` L47-158](../../src/console/install.rs#L47-L158) |

### 1.4 Binary architecture: native `aarch64` build vs `x86_64-unknown-linux-musl` under Rosetta

- `dist-workspace.toml` builds only two release targets ([`dist-workspace.toml` L12](../../dist-workspace.toml#L12)):
  ```toml
  targets = ["x86_64-unknown-linux-musl", "aarch64-apple-darwin"]
  ```
- Running `curl -LsSf https://thirdshift.app/install.sh | sh` inside an `aarch64` Linux VM on Apple Silicon will fail to find an `aarch64-unknown-linux-*` release archive.
- You have two choices inside an `aarch64` Linux VM:
  1. **Compile natively for `aarch64-unknown-linux-gnu` (`cargo install --path .` or `cargo install thirdshift`):** Uses the pinned Rust `1.99.1` toolchain ([`rust-toolchain.toml`](../../rust-toolchain.toml)) and produces a native ARM64 Linux binary. All system calls (`pidfd_open`, `bwrap`, `inotify`, `prctl`) and `rustc`/test executions run natively on the M5 Pro cores without translation.
  2. **Enable Apple Rosetta 2 for Linux (`VZLinuxRosettaDirectoryShare`)** ([Apple Developer Documentation](https://developer.apple.com/documentation/virtualization/vzlinuxrosettadirectoryshare)): When registered with `binfmt_misc`, the `aarch64` Linux kernel transparently executes `x86_64` ELF binaries—including `thirdshift-x86_64-unknown-linux-musl` from GitHub Releases or `x86_64`-only vendor tools—at near-native speed. Enabling Rosetta in the VM is still worthwhile even when compiling `thirdshift` natively, because agents or target repositories occasionally invoke `x86_64` prebuilt binaries.

### 1.5 Filesystem placement and `git` spawn performance

- Profiling in #846 and #894 ([`docs/ci-performance.md` L271-401](../ci-performance.md#L271-L401)) showed that `thirdshift` Runs and integration tests are heavily spawn- and metadata-bound:
  - A single `spec_run` test spawns up to **15,606 `git` processes**, spending 77–99% of wall time inside `git` (`git worktree list`, `git rev-parse --git-common-dir`, `git for-each-ref`).
  - A single `git rev-parse --git-common-dir` took **2.4 ms on Linux (`ext4`)** vs **9.1 ms (Xcode git) / 13.2 ms (Homebrew git) on macOS**, while macOS Spotlight (`mds`, `mdworker_shared`) consumed **18–35% of CPU** indexing temporary repositories until disabled.
- If repositories live on the macOS host (`/Users/jacob/...`) and are mounted into Linux via VirtioFS:
  1. Every `git` metadata operation crosses the VM boundary into the macOS host filesystem (APFS) and still triggers macOS Spotlight indexing unless Spotlight is disabled for that path.
  2. Linux `inotify` does not natively receive events for host-side changes on VirtioFS mounts ([Lima: Filesystem mounts — Mount Inotify](https://lima-vm.io/docs/config/mount/)).
  3. Unattended agent sessions (`claude -p --permission-mode auto`, `codex`, etc., [`docs/guide.md` L442-449](../guide.md#L442-L449)) can traverse whatever host directories are mounted into the VM.
- **Rule of thumb:** Keep all target repositories, worktrees, and `~/.thirdshift` inside the Linux VM's own `ext4` virtual disk (`/home/<user>/repos`), exactly as `thirdshift`'s user manual prescribes for WSL2 ([`docs/guide.md` L428](../guide.md#L428)).

### 1.6 Credentials and logins (`gh`, `claude`, `codex`, `agy`, `grok`, `muse`, `opencode`)

- On macOS, `gh auth login` and `claude` store OAuth credentials in the **macOS Keychain** by default. A Linux binary inside a VM cannot read the macOS Keychain even if `/Users/jacob` is mounted.
- Furthermore, `thirdshift`'s Trial sandbox ([`src/sandbox.rs` L740-802](../../src/sandbox.rs#L740-L802)) expects Linux file-based logins in the Linux user's `$HOME`:
  - **Claude Code:** `~/.claude.json` and `~/.claude/settings.json` (plus optional `claude_setup_token` in `~/.thirdshift/credentials.toml` or `CLAUDE_CODE_OAUTH_TOKEN`).
  - **Codex:** `~/.codex/auth.json` and `~/.codex/config.toml`.
  - **Antigravity CLI (`agy`):** `~/.gemini/antigravity-cli/antigravity-oauth-token`.
  - **Muse Code:** `~/.config/muse/auth.json`.
  - **Grok Build:** `~/.thirdshift/trials/grok/auth.json` (for Trials) and `~/.grok/`.
  - **OpenCode:** `opencode.api_key` in `~/.thirdshift/credentials.toml` (mode `0600`).
- Logging into `gh` (`gh auth login && gh auth setup-git`) and your chosen Harnesses directly inside the Linux VM creates clean, self-contained Linux credential files and avoids sharing your macOS home directory with unattended agents.

---

## 2. Options compared

All four options below run on Apple Silicon (`arm64`) on macOS Tahoe 26.5.2.

| Criterion | **Lima (`limactl`) + `vz`** *(Recommended)* | **OrbStack (`orb`) `--isolated`** | **Docker Desktop 29.8.2 container** | **Tart / UTM (`vz` Linux VM)** |
| --- | --- | --- | --- | --- |
| **Virtualization layer** | Apple `Virtualization.framework` (`vmType: vz`, default since Lima 1.0) ([Lima: VZ](https://lima-vm.io/docs/config/vmtype/vz/)) | Apple `Virtualization.framework` with custom VMM ([OrbStack: Architecture](https://docs.orbstack.dev/architecture)) | Docker VMM or Apple `Virtualization.framework` Linux VM hosting `dockerd` ([Docker: VMM](https://docs.docker.com/desktop/features/vmm/)) | Apple `Virtualization.framework` ([Tart](https://tart.run/), [UTM](https://docs.getutm.app/settings-apple/virtualization/)) |
| **Linux kernel** | Dedicated stock Ubuntu 24.04 kernel per VM (matches GitHub `ubuntu-24.04` runner) | Single custom OrbStack kernel shared across all machines and containers ([OrbStack: Isolated machines](https://docs.orbstack.dev/machines/isolated)) | Shared Linuxkit/Docker Desktop VM kernel | Dedicated distro kernel per VM |
| **`bwrap` (`--unshare-all`)** | Works natively after adding `/etc/apparmor.d/bwrap` ([`src/setup/sandbox.rs` L18-26](../../src/setup/sandbox.rs#L18-L26)) | Works natively (OrbStack kernel does not enforce Ubuntu's AppArmor `userns` block by default) | **Blocked by default** (`EPERM` on `unshare`/`clone`/`mount`/`pivot_root`); requires `--privileged` or unconfined seccomp/AppArmor + `CAP_SYS_ADMIN` ([Docker: Seccomp](https://docs.docker.com/engine/security/seccomp/)) | Works natively (with `/etc/apparmor.d/bwrap` on Ubuntu 24.04) |
| **`systemd` PID 1 & `systemd-run --user --scope`** | Out of the box (`ubuntu-24.04` cloud image boots `systemd` with cgroup v2) | Out of the box (OrbStack starts `systemd` and user manager automatically) ([OrbStack: Linux machines](https://docs.orbstack.dev/machines/)) | **Absent by default**; requires a custom `/sbin/init` image, `--privileged`, and cgroup v2 delegation | Out of the box on Ubuntu cloud/server images |
| **Host file isolation for unattended agents** | Configurable at creation (`--mount-none` or `--mount-only .:w`) ([Lima: AI agents](https://lima-vm.io/docs/examples/ai/)) | Default mounts `/Users` at `/mnt/mac`; `--isolated` disables host mounts and host network access ([OrbStack: Isolated machines](https://docs.orbstack.dev/machines/isolated)) | Isolated to explicit `-v` bind mounts unless `--privileged` is used (which exposes host VM devices) | Isolated by default unless `--dir` / VirtioFS share is configured |
| **Localhost port forwarding (`thirdshift console`)** | Automatic Guest `127.0.0.1:<port>` → macOS `localhost:<port>` via gRPC or `AF_VSOCK` SSH ([Lima: Port Forwarding](https://lima-vm.io/docs/config/port/)) | Automatic `localhost:<port>` and `<machine>.orb.local` with zero config ([OrbStack: Network](https://docs.orbstack.dev/machines/network)) | Requires explicit `-p <port>:<port>` at `docker run` creation time | Manual (`tart ip` or `--net-softnet-expose`; UTM `vz` lacks GUI port forwarding) ([UTM: Network](https://docs.getutm.app/settings-qemu/network/)) |
| **Rosetta 2 (`x86_64` ELF on `arm64`)** | Built-in (`--rosetta` sets `rosetta.enabled=true` and `rosetta.binfmt=true`) ([Lima: Multi-arch](https://lima-vm.io/docs/config/multi-arch/)) | Enabled automatically out of the box ([OrbStack: Architecture](https://docs.orbstack.dev/architecture)) | Supported on Apple Virtualization backend; **not supported on Docker VMM** ([Docker: VMM](https://docs.docker.com/desktop/features/vmm/#known-issues)) | Manual VirtioFS mount + `update-binfmts` inside guest ([UTM: Virtualization](https://docs.getutm.app/settings-apple/virtualization/)) |
| **RAM / CPU allocation** | Fixed ceiling per VM (e.g. `--cpus=12 --memory=24` on this 18-core / 64 GB Mac) | Dynamic memory allocation and release back to macOS ([OrbStack: Architecture](https://docs.orbstack.dev/architecture)) | Configured globally in Docker Desktop Settings | Fixed ceiling per VM |
| **License & cost** | Free, Apache-2.0 open source (`brew install lima`) | Free for personal use; **$8/user/month** ($96/yr) required for commercial/freelance use or >$10k/yr revenue ([OrbStack: Licensing](https://docs.orbstack.dev/licensing)) | Docker Personal or paid subscription (already installed) | Tart: Fair Source / free CLI; UTM: free open source (`brew install --cask utm`) |

---

### 2.1 Lima (`limactl`) with Apple `vz` and Ubuntu 24.04 LTS — Recommended

[Lima](https://lima-vm.io/) (Linux virtual machines, CNCF sandbox project, currently `2.2.1` in Homebrew) is purpose-built for running full Linux distributions on macOS with seamless CLI integration.

- **Native Apple `Virtualization.framework` (`vmType: vz`):** Since Lima v1.0, `vz` is the default VM driver on macOS ([Lima: VZ](https://lima-vm.io/docs/config/vmtype/vz/)). It boots an official Ubuntu 24.04 LTS (`noble`) `arm64` cloud image with its own kernel, `systemd` PID 1, `systemd-logind`, and unified cgroup v2 hierarchy.
- **Exact parity with `thirdshift`'s Ubuntu 24.04 target:** Because Lima boots Ubuntu's real kernel and userland rather than a container shim, `bwrap`, `/etc/apparmor.d/bwrap`, `systemd-run --user --scope`, `/sys/fs/cgroup/.../memory.events`, `pidfd_open`, and `/proc/self/exe` behave identically to a dedicated Ubuntu 24.04 server and to the repository's `ubuntu-24.04` CI runner ([`.github/workflows/ci.yml` L30-93](../../.github/workflows/ci.yml#L30-L93)).
- **Safe host isolation for AI agents:** By default, Lima mounts `~` read-only, and its AI agent documentation explicitly recommends restricting mounts (`--mount-none` or `--mount-only .:w`) when running autonomous coding agents so they cannot read or modify host files ([Lima: AI agents](https://lima-vm.io/docs/examples/ai/)).
- **Automatic localhost port forwarding:** Lima's guest agent monitors listening TCP sockets inside the VM and automatically forwards guest `127.0.0.1:<port>` to macOS `localhost:<port>` over gRPC or `AF_VSOCK` ([Lima: Port Forwarding](https://lima-vm.io/docs/config/port/)). That means:
  - `thirdshift console --no-sign-in` (or the `systemd` service installed by `thirdshift console install`) is immediately reachable at `http://localhost:<port>` in your macOS browser.
  - Browser-based OAuth logins (`gh auth login`, `claude`, `codex`, `agy`) started inside `limactl shell` can complete their `http://localhost:<callback>` redirect from your macOS browser without manual `ssh -L` port forwards.
- **Built-in Rosetta 2 registration:** Passing `--rosetta` mounts Apple's `VZLinuxRosettaDirectoryShare` and registers `/proc/sys/fs/binfmt_misc` automatically ([Lima: Intel-on-ARM](https://lima-vm.io/docs/config/multi-arch/)).

### 2.2 OrbStack (`orb`) Linux machines — Best low-overhead / UX alternative

[OrbStack](https://orbstack.dev/) (`2.2.5` in Homebrew Cask) provides both Docker containers and full-distro "Linux machines" (`orb create ubuntu:noble`).

- **How it works:** OrbStack runs a single lightweight Linux VM on `Virtualization.framework` and runs every Linux machine and Docker container on top of **one shared custom Linux kernel**, using namespaces and custom services written in Swift, Go, Rust, and C ([OrbStack: Architecture](https://docs.orbstack.dev/architecture)).
- **Why `--isolated` is mandatory for `thirdshift`:** By default, an OrbStack Linux machine mounts your entire macOS filesystem read-write at `/mnt/mac` (and symlinks `/Users/<user>`) and shares your macOS SSH agent ([OrbStack: File sharing](https://docs.orbstack.dev/machines/file-sharing)). Because `thirdshift` runs unattended agent sessions with approvals disabled ([`docs/guide.md` L442-449](../guide.md#L442-L449)), running in a default OrbStack machine exposes your entire Mac home directory to every non-Trial session. OrbStack's `--isolated` mode (`orb create --isolated ubuntu:noble thirdshift`) removes `/mnt/mac`, blocks connections to the macOS host network, and disables SSH agent forwarding while keeping `.orb.local` domains, `localhost` port forwarding, and `orb` CLI access ([OrbStack: Isolated machines](https://docs.orbstack.dev/machines/isolated)).
- **Trade-offs vs. Lima:**
  1. **Shared kernel:** As OrbStack's own documentation notes, "All machines and containers run in OrbStack's single Linux VM and share one kernel... For [a full security boundary], use a full virtual machine with its own kernel" ([OrbStack: Isolated machines — Security model](https://docs.orbstack.dev/machines/isolated#security-model)). Its custom kernel also differs from stock Ubuntu 24.04 (for instance, Ubuntu's AppArmor `userns` profile behavior is not reproduced identically).
  2. **Commercial licensing:** OrbStack is free only for personal/non-commercial use; professional, freelance, non-profit, government, or >$10,000/yr revenue use requires a paid Pro subscription at $8/user/month ([OrbStack: Licensing](https://docs.orbstack.dev/licensing)).

### 2.3 Why Docker Desktop containers (`docker run`) are the wrong fit

Docker `29.8.2` (`desktop-linux` context) is already installed on this MacBook, and `.github/workflows/release-smoke.yml` uses `docker run --rm ... rockylinux:9` to smoke-test `thirdshift version` ([`.github/workflows/release-smoke.yml` L64-67](../../.github/workflows/release-smoke.yml#L64-L67)). However, running full `thirdshift` workflows inside a Docker container hits three structural conflicts:

1. **Docker's default security profile blocks `bwrap`:**
   - Docker's default seccomp profile is an allowlist that blocks `unshare` (except `unshare --user`), `clone` namespace flags, `mount`, `umount`/`umount2`, and `pivot_root`, and drops `CAP_SYS_ADMIN` and `CAP_NET_ADMIN` ([Docker Engine: Seccomp security profiles](https://docs.docker.com/engine/security/seccomp/)).
   - Because `thirdshift`'s `bwrap` launcher creates PID, IPC, UTS, network, and mount namespaces (`--unshare-pid --unshare-ipc --unshare-uts --unshare-net --dev /dev --proc /proc --tmpfs /tmp --ro-bind ...`) and calls `pivot_root`, `bwrap` fails immediately inside an unprivileged container (`bwrap: Creating new namespace failed` / `Operation not permitted`).
2. **Docker containers do not run `systemd` or a user manager:**
   - `thirdshift` requires `systemd-run --user --scope`, `systemctl --user`, `journalctl --user`, and writable `/sys/fs/cgroup/.../memory.events` for every Trial and Grader ([`src/scope.rs` L147-456](../../src/scope.rs#L147-L456)), and `thirdshift console install` manages `/etc/systemd/system/thirdshift-console.service` ([`src/console/install.rs` L126-154](../../src/console/install.rs#L126-L154)).
   - Standard containers start a shell or single process as PID 1 without `systemd`, `dbus`, `systemd-logind`, or delegated user cgroups.
3. **Working around (1) and (2) requires `--privileged`:**
   - Booting `systemd` as PID 1 and permitting nested `bwrap` mounts/namespaces inside Docker requires a custom systemd-enabled image run with `--privileged --cgroupns=host -v /sys/fs/cgroup:/sys/fs/cgroup:rw`.
   - Running `--privileged` gives the container full root access to the underlying Docker Desktop VM, negating container isolation while still leaving you with ephemeral container state and manual `-p` port mappings.
   - Note also that if Docker Desktop is configured to use the beta **Docker VMM**, Docker's documentation notes that "Docker VMM does not currently support Rosetta, so emulation of amd64 architectures is slow" ([Docker Desktop: Virtual Machine Manager — Known issues](https://docs.docker.com/desktop/features/vmm/#known-issues)).

### 2.4 Tart and UTM

- **[Tart](https://tart.run/quick-start/)** (by Cirrus Labs) and **[UTM](https://docs.getutm.app/settings-apple/virtualization/)** both wrap Apple's `Virtualization.framework`, and either can boot an Ubuntu 24.04 ARM64 VM.
- For an interactive CLI + Console workflow like `thirdshift`, both require substantially more manual plumbing than Lima or OrbStack:
  - Neither automatically forwards guest `127.0.0.1:<port>` listeners to macOS `localhost:<port>`: UTM's port-forwarding UI is only available on its slower QEMU backend, not on its Apple Virtualization backend ([UTM: Network](https://docs.getutm.app/settings-qemu/network/)), so reaching `thirdshift console` or completing localhost OAuth callbacks requires looking up the guest IP or maintaining an explicit `ssh -L` tunnel.
  - Neither auto-registers Rosetta `binfmt_misc` or sets up passwordless guest shell entry (`limactl shell` / `orb`) out of the box on a raw Ubuntu cloud image ([UTM: Rosetta on Linux](https://docs.getutm.app/settings-apple/virtualization/)).

---

## 3. Step-by-step setup guide (Recommended: Lima + Ubuntu 24.04 LTS)

With 18 cores and 64 GB RAM on this M5 Pro MacBook, giving the Linux VM **12 vCPUs, 24 GiB RAM, and a 100 GiB disk** leaves 6 cores and 40 GiB RAM for macOS while giving `thirdshift` enough headroom to run parallel Spec Ticket runs (`spec.parallel = 3`), Rust builds, and 2 GiB-capped Trials without pressure.

### Step 1: Install Lima and ensure Rosetta 2 is present on macOS

On macOS:

```sh
brew install lima
# Needed only if Rosetta 2 is not yet installed on this Mac:
softwareupdate --install-rosetta --agree-to-license
```

### Step 2: Create an isolated Ubuntu 24.04 LTS (`vz`) VM

Start a named instance (`thirdshift`) from Lima's `ubuntu-24.04` template using Apple's `vz` hypervisor, Rosetta `binfmt_misc` support, `vzNAT` networking, and **no host filesystem mounts** (`--mount-none`) so all repositories and credentials live cleanly on the Linux `ext4` disk:

```sh
limactl start \
  --name=thirdshift \
  --vm-type=vz \
  --rosetta \
  --network=vzNAT \
  --cpus=12 \
  --memory=24 \
  --disk=100 \
  --mount-none \
  template:ubuntu-24.04
```

*(Optional)* Wire Lima into `~/.ssh/config` so `ssh lima-thirdshift` and VS Code Remote-SSH work directly ([Lima: SSH](https://lima-vm.io/docs/usage/ssh/)):

```sh
grep -q 'Include ~/.lima/\*/ssh.config' ~/.ssh/config 2>/dev/null || \
  printf '\nInclude ~/.lima/*/ssh.config\n' >> ~/.ssh/config
```

### Step 3: Install system packages, AppArmor `bwrap` profile, and systemd lingering

Enter the VM:

```sh
limactl shell thirdshift
```

Inside the Ubuntu 24.04 VM, install `thirdshift`'s runtime and build prerequisites (`bubblewrap`, `socat`, `gh`, `git`, `curl`, `build-essential`, `pkg-config`, `libssl-dev`) and Node.js 24 (matching `.github/workflows/ci.yml`, since Ubuntu 24.04's stock `nodejs` package is v18 and Playwright requires Node >= 20):

```sh
sudo apt update
sudo apt install -y \
  bubblewrap socat git gh curl ca-certificates xz-utils \
  build-essential pkg-config libssl-dev
curl -fsSL https://deb.nodesource.com/setup_24.x | sudo -E bash -
sudo apt install -y nodejs
```

Apply the Ubuntu 24.04 AppArmor profile that grants `/usr/bin/bwrap` unprivileged user namespaces ([`src/setup/sandbox.rs` L18-26](../../src/setup/sandbox.rs#L18-L26)):

```sh
sudo tee /etc/apparmor.d/bwrap <<'EOF'
abi <abi/4.0>,
include <tunables/global>
profile bwrap /usr/bin/bwrap flags=(unconfined) {
  userns,
  include if exists <local/bwrap>
}
EOF
sudo apparmor_parser -r /etc/apparmor.d/bwrap
```

Enable `systemd` user lingering so `systemd-run --user --scope` and scheduled passes (`cron` / user timers) work even when no interactive shell is open ([`src/scope.rs` L17-21](../../src/scope.rs#L17-L21)):

```sh
sudo loginctl enable-linger "$USER"
systemctl --user show -p SystemState --value
```

### Step 4: Install Rust and build `thirdshift` natively on `aarch64` Linux

Still inside the VM, install `rustup` (without modifying the default toolchain—`thirdshift`'s [`rust-toolchain.toml`](../../rust-toolchain.toml) will install `1.99.1` automatically) and install `thirdshift` from source on the native `ext4` filesystem:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
. "$HOME/.cargo/env"

mkdir -p ~/repos && cd ~/repos
git clone https://github.com/JacobStephens2/thirdshift.git
cd thirdshift
cargo install --path .
```

Ensure `~/.cargo/bin` and `~/.local/bin` are on `PATH` in `~/.bashrc` and `~/.profile`:

```sh
echo 'export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"' >> ~/.bashrc
echo 'export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"' >> ~/.profile
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
thirdshift version
```

*(Note on testing the `x86_64-unknown-linux-musl` release binary)*: Because `--rosetta` registered Apple's Rosetta 2 in `/proc/sys/fs/binfmt_misc`, you can also download and run the official `x86_64-unknown-linux-musl` release tarball directly inside this same `aarch64` VM whenever you want to smoke-test a release binary.

### Step 5: Authenticate `gh`, `git`, and your Harness CLI inside the VM

Because Lima forwards localhost ports between the guest and macOS, browser OAuth flows started inside the VM can be completed in your macOS browser:

```sh
# 1. Authenticate GitHub CLI and wire git credentials to it:
gh auth login
gh auth setup-git
git config --global user.name "Your Name"
git config --global user.email "you@example.com"

# 2. Install and sign in to your chosen Harness CLI(s), e.g. Claude Code / Codex:
curl -fsSL https://claude.ai/install.sh | bash
claude
```

### Step 6: Run `thirdshift setup` and verify the Linux sandbox probe

Inside the VM:

```sh
thirdshift setup
```

When `security.review` or `security.fix` is enabled, verify that Setup prints:

```text
Security sandbox probe: PASS (sandbox start, network connection blocked, host home read-only, scratch writable); sessions must still enforce every audit skill control
```

And to open the Console in your macOS browser:

```sh
thirdshift console --no-sign-in
```

Lima automatically forwards the printed `http://127.0.0.1:<port>` address to `http://localhost:<port>` on macOS. Or, if you want the Console to run continuously in the background inside the VM with zero sign-in:

```sh
thirdshift console port 8711
thirdshift console install --output ~/install-console.sh
sudo sh ~/install-console.sh
```

Then open `http://localhost:8711` on your MacBook at any time while the `thirdshift` Lima instance is running.

---

### Alternative quick setup: OrbStack isolated machine

If you prefer OrbStack's dynamic memory management and instant startup (and your usage fits its [personal or paid Pro license](https://docs.orbstack.dev/licensing)):

```sh
# On macOS:
brew install --cask orbstack
orb create --isolated ubuntu:noble thirdshift

# Enter the isolated Linux machine:
orb -m thirdshift
```

Inside `orb -m thirdshift`, follow **Steps 3–6** above (storing all clones under `/home/$USER/repos` inside the machine). Any port `thirdshift console` listens on is automatically forwarded to `http://localhost:<port>` and `http://thirdshift.orb.local:<port>` on macOS.
