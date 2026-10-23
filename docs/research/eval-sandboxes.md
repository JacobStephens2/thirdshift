# Sandboxes for Trials: bwrap, private networks, containers and microVMs

Research date: 2026-10-21 (America/New_York).

**Question.** Should the eval system run Trials, and perhaps Graders, inside Docker or another sandbox? And how would that change the decisions in the [#698 grilling](../../.scratch/grill-with-docs/done/698-evals/README.md)?

Q3.6 settled this much:
- each Trial gets a throwaway repository, checked for later commits;
- Harness memory is off;
- the Trial runs inside bwrap, which shows it only that repository and its Harness's login files.

The Settled list adds a risk. bwrap keeps the host's network unless given `--unshare-net`, so a Trial might reach a service on 127.0.0.1 ([eval-viewers.md §1.3](eval-viewers.md#13-putting-a-viewer-behind-a-sign-in)).

The question comes from Xander Song's AI Engineer NYC talk on Harbor and its Phoenix integration. The talk and the plugin covered the following, though the talk's Granola transcript never mentions `infra_ok`, which §7.5 places in the plugin:
- natural-language tasks;
- agents in fresh Docker environments;
- verifiers that grade what the agent leaves behind;
- rewards;
- an `infra_ok` that keeps a broken sandbox from counting as an agent failure;
- a plugin that feeds Arize Phoenix.

This note covers:
- the sandbox options for one agent CLI session per Trial on the `thirdshift` host (§1–§3), and how logins work inside them (§4);
- reproducible toolchains for the implement suite (§5);
- what Docker would open up (§6);
- how each option changes Q3.6, Q3.7, Q3.20, Q5.5, Q5.6 and the `infra_ok` idea (§7);
- a recommendation (§8).

It builds on [eval-runners.md](eval-runners.md) §1 and §4, [eval-viewers.md](eval-viewers.md) §1.3 and [eval-tools-landscape.md](eval-tools-landscape.md) §3.3, and doesn't repeat them.

**Method.**
- **The `thirdshift` host** was checked on 2026-10-21 with read-only commands:
  - `which`, `bwrap --version` and `sysctl`;
  - `cat` of AppArmor profiles, cgroup files and PAM files;
  - `systemctl cat` and `show`, and `loginctl show-user`;
  - the cgroup of each running `thirdshift` process;
  - `ss -ltn`, `free -m`, `df -h` and `du -sh`;
  - `dpkg-query`, `apt-cache policy`, `systemd-detect-virt` and `lsmod`.

  No Harness folder, Session log or replay file was read.
- **`stephens`** facts are the grilling's, from 2026-10-21. There was no SSH.
- **Sources:**
  - source at pinned commits;
  - man pages at stated versions;
  - official docs fetched on 2026-10-21;
  - registry metadata.

  Downloads went to the session's scratch folder and were read as data only.
- **Nothing was installed, started or run:** no container, daemon, VM or sandbox. No setting was changed.
- **Four read-throughs ran in parallel:**
  - Harbor and Terminal-Bench;
  - SWE-bench, Inspect and Langfuse;
  - the Linux mechanisms;
  - the Harnesses' sandboxes and logins.

  The claims that decide fit were then rechecked against the downloaded sources.

**Pins.** Citations link to these commits.

| Project | Version | Commit |
|---|---|---|
| bubblewrap | v0.9.0, the host's (2024-03-26); v0.13.1, the latest (2026-10-14) | `8e51677`; `362936a` |
| passt and pasta | `2024_02_20`, Ubuntu 24.04's; `2026_10_02`, the latest | `1e6f92b`; `cba3570` |
| slirp4netns | v1.2.1, Ubuntu 24.04's | `09e31e9` |
| Podman; containers/common | v4.9.3, Ubuntu 24.04's; v5.8.9; v0.57.4 | `8d2b55d`; `40030f4`; `215e898` |
| Docker docs; moby; rootlesskit | docs `main`; docker-v29.9.2; v3.3.0 | `226023f`; `0367cbe`; `888c318` |
| systemd | v255, the host's; v262.1 | `db11bab`; `73612ea` |
| gVisor | release-20261012.0 | `60faa65` |
| Firecracker; Cloud Hypervisor; Kata | v1.17.0; v54.0; 4.3.0 | `95f868c`; `038ce5a`; `bd75006` |
| Linux; Ubuntu's kernel | v6.8; Ubuntu-6.8.0-124.124, the running kernel | `e8f897f`; `97d6d07` |
| Codex | rust-v0.163.0 (2026-10-15) | `8308618` |
| sandbox-runtime | `main`, package 0.0.81 (2026-10-21) | `aceba12` |
| OpenCode | v1.19.3 (2026-10-16) | `e3c4eb2` |
| Harbor | v0.24.0 (2026-10-05); `main` (2026-10-21) | `b53b813`; `6c9d3d6` |
| Phoenix | `main`, client 3.5.0 (2026-10-21) | `f6a0231` |
| Terminal-Bench 2 | `main` (2026-04-30) | `2fd12b8` |
| SWE-bench | 5.0.2 (2026-08-18); v4.1.0 (2025-09-11) | `490635b`; `726c546` |
| swe-bench-tasks; swe-bench-multilingual-tasks | `main` | `3d07b46`; `6e08cbc` |
| Inspect AI; inspect_swe | 0.3.282 (2026-10-19); `main` (2026-10-21) | `386e099`; `377b111` |
| langfuse-docs; Langfuse; ClickHouse | `main`; v4.67.0; `master` (all 2026-10-21) | `5ee3f4e`; `21931f2`; `1b29cde` |
| Caddy's website | the docs for Caddy v2.11.7 | `1957519` |
| Linux man-pages | 6.19 on man7.org; 6.7 on kernel.org, Ubuntu 24.04's | `557f96f` for 6.7 |

**Tags.**
- **[interpretation]:** a judgment beyond what the source says.
- **[needs live check]:** only a real run would settle it.
- **[unverified]:** no adequate primary source was found.

## TL;DR

- **Keep bwrap as the boundary, and don't add Docker to the `thirdshift` host for Trials [interpretation].**
  - bwrap already shows a Trial only what it binds, and runs the same Harness binaries as a Run.
  - It needs no daemon or image, and it works with subscription logins, because the login file is bound in.
  - What Q3.6 left out is the network and the processes, and both fit inside bwrap (§3, §8).
- **Three gaps to close in Q3.6's sandbox:**
  - **/proc.** Without its own PID namespace, a Trial can read every host process's command line, and thirdshift passes Session prompts as arguments. `--unshare-pid` closes this. When the namespace ends, every process the Trial left behind is killed (§2).
  - **Host loopback.** It holds more than a future viewer: the Console on 8711, Caddy's admin API on 2019 (which Caddy's docs say to protect from untrusted code), and a dozen other listeners. `--unshare-net` leaves the sandbox only its own loopback (§1, §3.2).
  - **The public internet.** Seven of the eight repositories are public on GitHub, with their later fixes. Any network that reaches the internet reaches them, and only an allowlist closes that (§2, §3.2).
- **Two ways to reach the provider without the host's loopback** (§3.2):
  - **pasta joined to bwrap's namespaces,** with `--no-map-gw -T none -U none -t none -u none`.
    - Its defaults do the opposite. They map the gateway to the host's 127.0.0.1 and forward the host's loopback ports in. Its own man page shows `iperf3 -c 127.0.0.1` reaching the host.
    - Every Harness works unchanged, but GitHub and the host's public addresses stay reachable.
    - Ubuntu 24.04's passt package lacks pasta's AppArmor profile **[needs live check]**.
  - **An allowlisting proxy over a Unix socket.** This is how Anthropic's sandbox-runtime works, and what its Agent SDK guidance shows.
    - The namespace has no route out, and only the provider's hosts are allowed. That closes all three leaks.
    - Each Harness must honour `HTTPS_PROXY`. Claude Code's and Grok's docs say they do, and Codex's imply it; the other three need a live check.
- **Memory (Q3.7): cgroups can cap Trials, but not as the host is set up today.**
  - The factory runs in `cron.service`, which jacob can't subdivide. Lingering is off, so a cron job has no user manager to ask for a scope.
  - One `loginctl enable-linger jacob` makes a scope per Trial possible, with `MemoryHigh` and `MemoryMax`. Rootless Podman needs the same step.
  - Until then, raising `oom_score_adj` makes Trials the OOM killer's first choice (§7.2).
- **Logins:** bind the one real login file. Never bind a copy, and never a whole Harness home (§4).
  - Codex writes `auth.json` in place and rotates refresh tokens. A copy that refreshes leaves the factory's file holding a used token, and Harbor's Codex agent uploads exactly such a copy.
  - Claude can skip the file: `claude setup-token` makes a one-year subscription token for `CLAUDE_CODE_OAUTH_TOKEN`.
  - Harness homes hold session histories, including the Graders' and the ones that wrote the later fixes.
- **Containers fit worse on this host** (§3.3–§3.8):
  - **Docker:** the docker group is root-equivalent, and every unattended factory session runs as jacob.
  - **Rootless Podman** has no daemon and blocks host loopback by default. But its limits need lingering, and each repository needs an image holding six Harnesses.
  - **The rest:**
    - nspawn needs root on systemd 255;
    - rootless gVisor offers only the host's network, or none;
    - Firecracker needs root for its jailer and its network, and has no shared filesystem for a refreshed login.
  - **The Harnesses' own sandboxes** confine commands, not the session. Most leave the disk readable, and ADR 0013 turns them off.
- **Docker would make Harbor, Inspect's sandbox and SWE-bench's harness usable, but none of them runs a Trial through thirdshift on a subscription** (§6).
  - Harbor's built-in agents start sessions their own way, and four of the six that match thirdshift's Harnesses need API keys.
  - inspect_swe routes model calls through Inspect's API-key providers.
  - SWE-bench grades patches.
  - Harbor's telemetry is on by default.
  - Worth copying: SWE-bench's history scrub, and Harbor's failure causes and network modes.
- **`infra_ok` isn't Harbor's** (§7.5).
  - It is a score in Phoenix's plugin. It is 0 for any recorded exception, timeouts included, and the reward is kept either way.
  - Harbor's metrics count such trials as 0. SWE-bench flags infrastructure failures but keeps them in its denominator.
  - A **void** Trial in thirdshift should follow rules fixed with the bar. It is re-run, shown, and kept out of Q4.4's interval.
- **The implement suite (Q3.20)** should start on the host's toolchains, in bwrap, with SWE-bench's history scrub and with code Graders that have no network at all.
  - Per-repository images would need disk this host doesn't have: 32 GB is free, and SWE-bench asks for 120 GB.
  - They would also need a rebuild every time production's Harness versions change (§5, §7.4).
- **Q5.5:** Langfuse still asks for 16 GiB, and ClickHouse recommends 32 GB. `stephens` has 15,993 MiB with swap in use. No sandbox choice changes that (§6.4).
- **Q5.6:** a container sandbox gives Inspect a container lifecycle and makes Harbor runnable. It doesn't give either one subscription Trials through thirdshift, so (a) stands (§7.3).

## 1. The `thirdshift` host, checked on 2026-10-21

- **Machine.**
  - Ubuntu 24.04.4 LTS, kernel 6.8.0-124-generic, 4 cores.
  - A KVM guest: `systemd-detect-virt` prints `kvm`, and the CPU model is "DO-Regular".
- **Memory.** 7,941 MiB in all. At the time, 3,791 MiB was available and 1.8 GiB of the 4 GiB swap file was in use (`free -m`, `swapon --show`).
- **Disk.**
  - 154 GB, with 32 GB free and 80% used (`df -h /`).
  - The filesystem is ext4, so copies can't share blocks through reflinks (`findmnt`).
  - Build output takes the most space:
    - thirdshift's `target/` is 6.0 GB;
    - the four retained thirdshift worktrees' `target/` folders are 1.6–2.7 GB each;
    - `~/.rustup` is 2.6 GB, `~/.npm` 2.0 GB, `~/.cargo/registry` 1.1 GB and `~/.nvm` 773 MB (`du -sh`).
- **Toolchains.**
  - rustc 1.99.0, which thirdshift's `rust-toolchain.toml` pins, plus stable.
  - Node 26.10.0 under nvm, Python 3.12.3, and PHP 8.3.6 without Composer.
  - The seven repositories in this host's crontab:
    - thirdshift, cascade and vaulted-agent are Rust;
    - chart35 and clave are Node;
    - muxboard is Python;
    - keeplore is PHP, with a `docker-compose.yml`.
- **bwrap.**
  - bubblewrap 0.9.0 (package `0.9.0-1ubuntu0.3`), with `kernel.apparmor_restrict_unprivileged_userns = 1`.
  - `/etc/apparmor.d/bwrap`, dated 9 October, reads `profile bwrap /usr/bin/bwrap flags=(unconfined) { userns, }`.
  - It is the profile Claude Code's docs give for Ubuntu 24.04 ([sandboxing](https://code.claude.com/docs/en/sandboxing#set-up-linux-and-wsl2)).
- **Other user-namespace profiles.**
  - Ubuntu's own profiles grant `userns`, by path, to `/usr/bin/podman`, `/usr/bin/crun`, `/usr/sbin/runc`, `/usr/bin/slirp4netns`, `/usr/bin/rootlesskit` and `/usr/bin/buildah`. None covers pasta.
  - Any other program that creates a user namespace lands in `/etc/apparmor.d/unprivileged_userns`, which has `audit deny capability`.
  - Ubuntu's release notes describe the result: such programs may create the namespace, but "the subsequent use of any capabilities" is denied ([24.04 release notes](https://discourse.ubuntu.com/t/noble-numbat-release-notes/39890)).
  - The running kernel's AppArmor hook covers creating a user namespace only ([task.c n424-n446](https://git.launchpad.net/~ubuntu-kernel/ubuntu/+source/linux/+git/noble/tree/security/apparmor/task.c?id=97d6d07ae111bd2656537f792bae578399e75c7a#n424)).
- **Not installed:**
  - Docker, Podman, pasta and passt, slirp4netns, and `uidmap` (newuidmap and newgidmap);
  - crun, runc, containerd and runsc;
  - `systemd-container` (nspawn), Firecracker and QEMU.

  socat 1.8.0.0 is installed.
- **What apt offers** (`apt-cache policy`):
  - Podman 4.9.3, passt `0.0~git20240220.1e6f92b`, slirp4netns 1.2.1, `uidmap` and `systemd-container` 255.4;
  - `docker.io` 29.1.3;
  - `docker-ce` 29.9.2 from download.docker.com, whose source list is already configured.
- **Subordinate IDs.** `/etc/subuid` and `/etc/subgid` already give `jacob` 231072:65536.
- **cgroups.**
  - cgroup v2 under systemd 255.4.
  - `user@.service` has `Delegate=pids memory cpu`, and `user@1002.service` delegates `cpu memory pids`.
  - Lingering is off for `jacob` (`loginctl show-user`).
- **Where the factory runs.**
  - The five running `thirdshift` processes were in `/system.slice/cron.service`, and the Console in `thirdshift-console.service`.
  - cron's PAM stack includes `common-session-noninteractive`, which has no `pam_systemd`. So a cron job gets no login session and starts no user manager.
  - `cron.service` has `MemoryMax=infinity` and `MemoryHigh=infinity`.
- **Listeners on loopback** (`ss -ltn`):
  - the Console on 127.0.0.1:8711;
  - Caddy's admin API on 127.0.0.1:2019;
  - systemd-resolved on 127.0.0.53 and 127.0.0.54;
  - a service on 8100, and nine others whose owners weren't checked.

  Caddy also listens on all addresses at 80 and 443, and sshd at 22.
- **What Caddy says about its admin API.** Its "Default address" is `localhost:2019`. For servers running untrusted code it says to "make sure you protect your admin endpoint by isolating processes, patching vulnerable programs, and configuring the endpoint to bind to a permissioned unix socket instead" ([api.md L9-L14](https://github.com/caddyserver/website/blob/19575198cce1ea5f1bb37895ef3f9f2ec0f1997a/src/docs/markdown/api.md#L9-L14)).
- **DNS.** `/etc/resolv.conf` names 127.0.0.53.
- **Virtualization.**
  - `/dev/kvm` exists, owned `root:kvm` with mode 660, and `jacob` isn't in group `kvm`. Its groups are `jacob` and `sudo`.
  - `kvm_intel` is loaded with `nested=Y`, and the CPU shows `vmx`.
- **Terminal injection.**
  - `dev.tty.legacy_tiocsti = 0`, so pushing keystrokes into a terminal needs `CAP_SYS_ADMIN` ([drivers/tty/Kconfig L152-L172](https://github.com/torvalds/linux/blob/e8f897f4afef0031fe618a8e94127a0934896aba/drivers/tty/Kconfig#L152-L172)).
  - A Trial started from Jacob's terminal can't type into it.
- **/proc** is mounted without `hidepid`. Yama's `ptrace_scope` is 1.

## 2. What a Trial's sandbox has to do

From the grilling:
- **Files.**
  - The Trial sees its Eval task's repository (read-write), the Factory skills linked into it, and its Harness's login.
  - It must not see `~/.thirdshift/replays/`, the real checkouts that hold later fixes, other Trials' folders, or the Graders' packets (Q3.6).
- **Host loopback.** Unreachable (Settled, "Trials and the host's network").
- **The provider.** Reachable, as in a Run (Q3.12's check).
- **Processes.** Everything a Trial starts ends with it. eval-runners.md found that Inspect signals only a Trial's direct child, and that promptfoo's and Mira's timeouts leave sessions running (§4.2, §8.1 there).
- **Memory.** Two Trials beside about two building Runs, on 8 GB (Q3.7).
- **Logins.** A refreshed token must reach the one file production uses (§4).
- **Fidelity.**
  - The same Harness binary runs with ADR 0013's flags, started by thirdshift's own code (Q3.8), on the same userland.
  - Q3.6's recommendation put it as "It differs from a production Run only in what the session can see on disk."
- **Graders.**
  - **Model Graders** need the same, with a packet in place of a repository.
  - **Code Graders** in the implement suite run tests on code an agent wrote. The Security skill already says such code runs only with "no external network", an allowlisted environment, a read-only toolchain, and "explicit low CPU, memory, process, file-size, disk, and wall-clock limits" ([SKILL.md L33-L38](../../skills/thirdshift-security-audit/SKILL.md)).

Two gaps follow from the host's facts:
- **/proc leaks command lines.** Without its own PID namespace, a Trial can read every host process's command line.
  - /proc is mounted without `hidepid`, and `/proc/<pid>/cmdline` has no permission check ([proc(5)](https://man7.org/linux/man-pages/man5/proc.5.html), `hidepid`; [proc_pid_cmdline(5)](https://man7.org/linux/man-pages/man5/proc_pid_cmdline.5.html)).
  - thirdshift passes each Session prompt as an argument (the grilling's 2026-10-16 results). So a concurrent production session's prompt, or a Grader's, would be readable **[interpretation]**.
  - `/proc/<pid>/cwd` and `environ` need a ptrace read check ([proc_pid_cwd(5)](https://man7.org/linux/man-pages/man5/proc_pid_cwd.5.html), [proc_pid_environ(5)](https://man7.org/linux/man-pages/man5/proc_pid_environ.5.html)). That check fails from inside a new user namespace unless the caller holds `CAP_SYS_PTRACE` in the target's namespace ([ptrace(2)](https://man7.org/linux/man-pages/man2/ptrace.2.html), "Ptrace access mode checking", step 5.2).
  - So bwrap's user namespace already blocks `cwd` and `environ`, but not `cmdline` **[interpretation]**.
  - `--unshare-pid` with a fresh `/proc` closes it. When the namespace's init dies, "the kernel terminates all of the processes in the namespace via a SIGKILL signal" ([pid_namespaces(7)](https://man7.org/linux/man-pages/man7/pid_namespaces.7.html)), so a Trial's orphans die with it too.
- **The answers are on GitHub.**
  - Seven of the eight repositories the factory works on are public (the grilling's 2026-10-16 results), and their later fixes are public with them.
  - A Trial with no `origin` and `gh` signed out can still `git clone` or `curl` them **[interpretation]**. #429 relied on its prompt here.
  - Only a host allowlist stops that (§3.2).
  - Nothing outside the Harness stops a search the provider runs on its own side. Codex's docs say its network proxy "does not filter web search" ([agent-approvals-security](https://learn.chatgpt.com/docs/agent-approvals-security#traffic-outside-the-command-network-proxy)). Whether Claude's WebSearch runs on Anthropic's side is **[unverified]**.

## 3. The options

Each option is judged for one agent CLI session per Trial. §3.9 puts them side by side.

### 3.1 bwrap as Q3.6 settled it

Q3.6 settled file binds only, sharing the host's network and PID namespace.

- **Files.**
  - bwrap always makes a new mount namespace whose root is "on a tmpfs that is invisible from the host" ([bwrap.xml L49-L53](https://github.com/containers/bubblewrap/blob/8e51677abd7e3338e4952370bf7d902e37d8cbb6/bwrap.xml#L49-L53)). The Trial sees only what is bound.
  - Unprivileged bwrap's user namespace "hides all but the current uid and gid" ([README L158-L159](https://github.com/containers/bubblewrap/blob/8e51677abd7e3338e4952370bf7d902e37d8cbb6/README.md#L158-L159)).
- **Loopback and network: shared.** The Trial reaches every listener of §1. It also reaches the host's abstract Unix sockets:
  - network namespaces "isolate the UNIX domain abstract socket namespace" ([network_namespaces(7) n21-n23](https://git.kernel.org/pub/scm/docs/man-pages/man-pages.git/tree/man7/network_namespaces.7?id=557f96fb1938cc82a543ae5af1dec920c282cf1e#n21));
  - "Socket permissions have no meaning for abstract sockets" ([unix(7) n253-n260](https://git.kernel.org/pub/scm/docs/man-pages/man-pages.git/tree/man7/unix.7?id=557f96fb1938cc82a543ae5af1dec920c282cf1e#n253)).

  Podman's docs warn the same about host networking, which gives "full access to abstract Unix domain sockets and to TCP/UDP sockets bound to localhost" ([network.md L30](https://github.com/containers/podman/blob/40030f4073bd4e1bf4f2fd9df2f6952b4d9d350a/docs/source/markdown/options/network.md#L30)).
- **Processes.**
  - Without `--unshare-pid`, the Trial sees the host's command lines (§2).
  - With it, "The sandbox will not see any processes outside the sandbox", and bwrap runs a reaper as pid 1 ([README L164](https://github.com/containers/bubblewrap/blob/8e51677abd7e3338e4952370bf7d902e37d8cbb6/README.md#L164)).
  - `--die-with-parent` SIGKILLs the sandbox when bwrap or its parent dies ([bwrap.xml L486-L493](https://github.com/containers/bubblewrap/blob/8e51677abd7e3338e4952370bf7d902e37d8cbb6/bwrap.xml#L486-L493)).
- **cgroups: none.**
  - bwrap has only `--unshare-cgroup`, which makes a cgroup namespace ([bwrap.xml L127-L132](https://github.com/containers/bubblewrap/blob/8e51677abd7e3338e4952370bf7d902e37d8cbb6/bwrap.xml#L127-L132)).
  - Neither 0.9.0 nor 0.13.1 has any rlimit or cgroup-limit code (a grep of both sources).
- **Missing in 0.9.0.** Overlay mounts arrived in 0.11.0, released 2024-10-30 ([NEWS.md L157-L176](https://github.com/containers/bubblewrap/blob/362936aea0d5c6e70f7af75d2d44b5a914cb4ba2/NEWS.md#L157-L176)). Ubuntu 24.04 ships 0.9.0.
- **Setup and root.** Setup is done: the AppArmor profile is in place. Starting a sandbox needs no root.
- **Overhead.** No primary figures were found **[unverified]**. It adds a process and needs no image **[interpretation]**.

### 3.2 bwrap with a network namespace of its own

`--unshare-net` gives the sandbox "its own network namespace with only a loopback device" ([README L167](https://github.com/containers/bubblewrap/blob/8e51677abd7e3338e4952370bf7d902e37d8cbb6/README.md#L167)). The Harness then needs a way out to its provider. There are three.

**(a) pasta**
- **It can join bwrap's namespaces.**
  - "If PID or --netns are given, pasta associates to an existing user and network namespace" ([passt.1 n64-n67](https://passt.top/passt/tree/passt.1?id=1e6f92b995a9f7bff83b8b7a4b51782804fc23c5#n64)).
  - Given a PID, it calls setns() on that process's user namespace and creates none ([conf.c n507-n526](https://passt.top/passt/tree/conf.c?id=1e6f92b995a9f7bff83b8b7a4b51782804fc23c5#n507), [isolation.c n247-n265](https://passt.top/passt/tree/isolation.c?id=1e6f92b995a9f7bff83b8b7a4b51782804fc23c5#n247)).
  - bwrap writes the child's PID, as seen "in the pid namespace from which bwrap was run", to `--json-status-fd`. `--block-fd` holds the sandbox until data arrives ([bwrap.xml L430-L470](https://github.com/containers/bubblewrap/blob/8e51677abd7e3338e4952370bf7d902e37d8cbb6/bwrap.xml#L430-L470)).
  - So thirdshift can start pasta on that PID and release the Harness once the network is up **[interpretation]**.
- **Its defaults expose the host's loopback in two ways.**
  - **The gateway is mapped to the host's loopback.** `--map-host-loopback` makes packets "appear to have both source and destination of 127.0.0.1", and the default is "to translate the guest's default gateway address, unless --no-map-gw is given" ([passt.1 n376-n387](https://passt.top/passt/tree/passt.1?id=cba357068dc586e7540971eb40bb56c5e62c4de1#n376); the 2024 `--no-map-gw` at [n305-n308](https://passt.top/passt/tree/passt.1?id=1e6f92b995a9f7bff83b8b7a4b51782804fc23c5#n305)).
  - **The host's ports are forwarded in.** `-T` and `-U`, which forward "from target namespace to init namespace", default to `auto`, which forwards whatever is bound ([passt.1 n441-n446, n520-n531](https://passt.top/passt/tree/passt.1?id=1e6f92b995a9f7bff83b8b7a4b51782804fc23c5#n441)).
  - The man page's own example runs `iperf3 -s` on the host, then `iperf3 -c 127.0.0.1` inside a fresh namespace, and connects ([n600-n646](https://passt.top/passt/tree/passt.1?id=1e6f92b995a9f7bff83b8b7a4b51782804fc23c5#n600)).
  - For pasta, `-t` and `-u` also default to `auto` ([n565-n577](https://passt.top/passt/tree/passt.1?id=cba357068dc586e7540971eb40bb56c5e62c4de1#n565)), which would publish the Trial's own listeners on the host.
- **The settings that close the loopback** are the ones Podman and rootlesskit use: `--no-map-gw -T none -U none`, plus `-t none -u none` ([Podman 4.9.3 network.md L49-L59](https://github.com/containers/podman/blob/8d2b55ddde1bc81f43d018dfc1ac027c06b26a7f/docs/source/markdown/options/network.md#L49-L59); [rootlesskit pasta.go L177-L179](https://github.com/rootless-containers/rootlesskit/blob/888c3189e9cdf34dcdb19011e1c7c34eb94b5393/pkg/network/pasta/pasta.go#L177-L179)). DNS then needs `--dns-forward`, because this host's resolver is on loopback ([conf.c n114-n138](https://passt.top/passt/tree/conf.c?id=cba357068dc586e7540971eb40bb56c5e62c4de1#n114)) **[needs live check]**.
- **What stays open [interpretation]:**
  - the internet, GitHub included;
  - the host's own non-loopback addresses. That covers anything bound to all interfaces: Caddy's 443, and a viewer like `promptfoo view`, which "listens on every network interface" (grilling, Q3.12).
- **Setup.**
  - `apt install passt`. Ubuntu 24.04's package installs a profile for `/usr/bin/passt` only, with no `usr.bin.pasta` ([LP #2077158](https://bugs.launchpad.net/ubuntu/+source/passt/+bug/2077158)).
  - rootlesskit's docs say this version "doesn't work … due to a missing `usr.bin.pasta` AppArmor profile", and suggest adding upstream's ([network.md L153-L165](https://github.com/rootless-containers/rootlesskit/blob/888c3189e9cdf34dcdb19011e1c7c34eb94b5393/docs/network.md#L153-L165)).
  - Joining an existing user namespace needs only `CAP_SYS_ADMIN` in it, which the owner's euid has ([user_namespaces(7) n120-n135](https://git.kernel.org/pub/scm/docs/man-pages/man-pages.git/tree/man7/user_namespaces.7?id=557f96fb1938cc82a543ae5af1dec920c282cf1e#n120); [user_namespace.c L1339-L1365](https://github.com/torvalds/linux/blob/e8f897f4afef0031fe618a8e94127a0934896aba/kernel/user_namespace.c#L1339-L1365)). The AppArmor restriction covers creating one (§1).
  - So attaching may work without a profile **[interpretation; needs live check]**.
- **Overhead.** One pasta process per Trial. Its README claims "no dynamic memory allocation" ([README n288-n323](https://passt.top/passt/tree/README.md?id=cba357068dc586e7540971eb40bb56c5e62c4de1#n288)). Its memory use is **[unverified]**.

**(b) slirp4netns**
- **It joins a PID's namespaces the same way** and creates no user namespace ([slirp4netns.1.md L61-L65](https://github.com/rootless-containers/slirp4netns/blob/09e31e92fa3d2a1d3ca261adaeb012c8d75a8194/slirp4netns.1.md#L61-L65); [main.c L41-L80](https://github.com/rootless-containers/slirp4netns/blob/09e31e92fa3d2a1d3ca261adaeb012c8d75a8194/main.c#L41-L80)).
- **Host loopback is open by default** ([L194-L206](https://github.com/rootless-containers/slirp4netns/blob/09e31e92fa3d2a1d3ca261adaeb012c8d75a8194/slirp4netns.1.md#L194-L206)):
  - "ports listening on `INADDR_LOOPBACK` (`127.0.0.1`) on the host are accessible from the child namespace via the gateway (default: `10.0.2.2`)";
  - `--disable-host-loopback` prohibits this;
  - but "a host loopback address might be still accessible via the built-in DNS (default: `10.0.2.3`) if `/etc/resolv.conf` on the host refers to a loopback address", as this host's does.

  That leaves port 53 of the stub resolver **[interpretation]**.
- Ubuntu ships its AppArmor profile (§1). Its throughput is lower than pasta's in rootlesskit's iperf3 comparison ([network.md L11-L22](https://github.com/rootless-containers/rootlesskit/blob/888c3189e9cdf34dcdb19011e1c7c34eb94b5393/docs/network.md#L11-L22)).

**(c) An allowlisting proxy over a Unix socket**
- **How it works.**
  - The namespace keeps only its loopback.
  - A proxy outside the sandbox listens on a Unix socket bound into it, and a relay inside turns a local TCP port into that socket.
  - The Harness reaches its provider through `HTTPS_PROXY`.
  - The proxy allows listed hosts only, and refuses names that resolve to loopback or to the host's own addresses.
- **Anthropic uses this design for unattended agents.**
  - sandbox-runtime (Apache-2.0, a "Beta Research Preview") runs bwrap with `--unshare-net`, socat on both sides of the socket, and `HTTP(S)_PROXY` pointing at `localhost:3128` ([README L1-L9](https://github.com/anthropics/sandbox-runtime/blob/aceba124fd725f4a43d7016ed3f5651d61f83115/README.md#L1-L9); [linux-sandbox-utils.ts L1513-L1530](https://github.com/anthropics/sandbox-runtime/blob/aceba124fd725f4a43d7016ed3f5651d61f83115/src/sandbox/linux-sandbox-utils.ts#L1513-L1530), [L1793-L1830](https://github.com/anthropics/sandbox-runtime/blob/aceba124fd725f4a43d7016ed3f5651d61f83115/src/sandbox/linux-sandbox-utils.ts#L1793-L1830); [sandbox-utils.ts L820-L870](https://github.com/anthropics/sandbox-runtime/blob/aceba124fd725f4a43d7016ed3f5651d61f83115/src/sandbox/sandbox-utils.ts#L820-L870)).
  - It refuses allowlisted names that resolve to loopback or to the host's addresses ([resolved-address-guard.ts L14-L23](https://github.com/anthropics/sandbox-runtime/blob/aceba124fd725f4a43d7016ed3f5651d61f83115/src/sandbox/resolved-address-guard.ts#L14-L23)).
  - Anthropic's Agent SDK guidance shows `--network none`, with "a Unix socket connected to a proxy running outside the container" ([secure-deployment](https://code.claude.com/docs/en/agent-sdk/secure-deployment)).
  - Codex's managed proxy mode uses "`--unshare-net` plus an internal TCP->UDS->TCP routing bridge" ([linux-sandbox README L91-L101](https://github.com/openai/codex/blob/83086188ae0614bd882818ff1aa23f3194962012/codex-rs/linux-sandbox/README.md#L91-L101)).
- **It closes all three leaks:** loopback, the host's public addresses, and GitHub. For Trials it also enforces Q2.2's rule that nothing leaves the host but the sessions **[interpretation]**.
- **It depends on each Harness honouring the proxy variables.**
  - sandbox-runtime "Currently uses environment variables (`HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`)", which "may be ignored by programs that don't respect these variables" ([README L1284](https://github.com/anthropics/sandbox-runtime/blob/aceba124fd725f4a43d7016ed3f5651d61f83115/README.md#L1284)).
  - **Claude Code** supports `HTTPS_PROXY`, but not SOCKS ([network-config](https://code.claude.com/docs/en/network-config#network-access-requirements)).
  - **Grok** "honors standard proxy environment variables" ([enterprise](https://docs.x.ai/build/enterprise)).
  - **Codex** sends its model and authentication requests through "the client's separate HTTP and system-proxy settings" ([permissions](https://learn.chatgpt.com/docs/permissions)).
  - **OpenCode, Muse and agy** are **[unverified]**.
  - A Harness that ignores the variables finds no route and fails. The Trial is then void, not leaky **[interpretation]**.
- **It also blocks production-like web use** from inside the session: `curl`, `git clone`, and Claude's WebFetch. #429 already turned off Claude's WebFetch and WebSearch in its Trials (the grilling's 2026-10-18 results).
- **Using sandbox-runtime itself** would bring Node and its defaults:
  - "read access is allowed everywhere" ([README L119-L126](https://github.com/anthropics/sandbox-runtime/blob/aceba124fd725f4a43d7016ed3f5651d61f83115/README.md#L119-L126)), the opposite of Q3.6's bind list;
  - a seccomp helper that needs "capability-bearing user namespaces", which Ubuntu 24.04's restriction strips ([README L701](https://github.com/anthropics/sandbox-runtime/blob/aceba124fd725f4a43d7016ed3f5651d61f83115/README.md#L701)) **[needs live check]**.
- **Writing it instead.** socat is installed. A CONNECT proxy with an allowlist and an address check is a few hundred lines of blocking Rust **[interpretation]**.

### 3.3 Rootless Podman

- **Isolation.**
  - The image's filesystem, plus binds.
  - Host loopback is closed by default on both of its networks:
    - in 4.9, slirp4netns's `allow_host_loopback`: "The default is false" ([4.9.3 network.md L27-L28](https://github.com/containers/podman/blob/8d2b55ddde1bc81f43d018dfc1ac027c06b26a7f/docs/source/markdown/options/network.md#L27-L28));
    - in 5.x, pasta with `--no-map-gw` and `-t/-u/-T/-U none` ([5.8.9 network.md L52-L74](https://github.com/containers/podman/blob/40030f4073bd4e1bf4f2fd9df2f6952b4d9d350a/docs/source/markdown/options/network.md#L52-L74)).
  - 5.x maps `host.containers.internal` to the host's global address.
- **Setup here.**
  - `apt install podman uidmap slirp4netns`.
  - Ubuntu already ships `userns` profiles for podman, crun and slirp4netns, and jacob's subordinate IDs exist (§1).
  - Rootless Podman needs newuidmap and subuid ranges ([rootless_tutorial.md L17-L25](https://github.com/containers/podman/blob/8d2b55ddde1bc81f43d018dfc1ac027c06b26a7f/docs/tutorials/rootless_tutorial.md#L17-L25)).
- **No daemon.** Podman is "a simple daemonless tool" ([podman.1.md L10](https://github.com/containers/podman/blob/8d2b55ddde1bc81f43d018dfc1ac027c06b26a7f/docs/source/markdown/podman.1.md#L10)).
- **cgroup limits need a systemd user session.**
  - Rootless cgroup v2 "requires the --cgroup-manager … to use systemd" ([rootless.md L16-L18](https://github.com/containers/podman/blob/8d2b55ddde1bc81f43d018dfc1ac027c06b26a7f/rootless.md#L16-L18)).
  - Without a session bus, Podman falls back to cgroupfs ([config.go L690-L713](https://github.com/containers/common/blob/215e898615b7de5f789be60784b8303f2ab6de3d/pkg/config/config.go#L690-L713)).
  - "Podman expects a valid login session for the rootless+cgroupv2 use-case", and its troubleshooting suggests lingering ([troubleshooting.md L955-L979](https://github.com/containers/podman/blob/8d2b55ddde1bc81f43d018dfc1ac027c06b26a7f/troubleshooting.md#L955-L979)).
  - From cron, with lingering off, limits wouldn't apply **[interpretation; needs live check]**.
- **Logins.** `--userns=keep-id` maps jacob's UID and GID to the same values ([userns.container.md L25](https://github.com/containers/podman/blob/8d2b55ddde1bc81f43d018dfc1ac027c06b26a7f/docs/source/markdown/options/userns.container.md#L25)), so a bound login file stays jacob's.
- **Overhead.** conmon per container, plus image storage. No primary figures were found **[unverified]**.
- **Fit [interpretation].** It is the cleanest container option for this host. But each image needs the six Harnesses at production's versions, plus thirdshift itself (§5).

### 3.4 Docker

**Rootful**
- **Privilege.**
  - "The `docker` group grants root-level privileges to the user" ([linux-postinstall.md L35-L39](https://github.com/docker/docs/blob/226023fc89bdeb3f051c4c8c697d04ed5f5bcd2c/content/manuals/engine/install/linux-postinstall.md#L35-L39)).
  - Every unattended factory session runs as jacob with the account's full permissions (ADR 0013). So adding jacob to that group hands them all passwordless root **[interpretation]**.
- **Firewall.** Ports Docker publishes bypass ufw ([packet-filtering-firewalls.md L90-L102](https://github.com/docker/docs/blob/226023fc89bdeb3f051c4c8c697d04ed5f5bcd2c/content/manuals/engine/network/packet-filtering-firewalls.md#L90-L102)).
- **Host loopback.**
  - On a bridge network, the container's 127.0.0.1 is its own **[interpretation]**.
  - `host-gateway` still reaches host services bound to all interfaces ([docker_container_run.yaml L2274-L2296](https://github.com/docker/docs/blob/226023fc89bdeb3f051c4c8c697d04ed5f5bcd2c/data/cli/engine/docker_container_run.yaml#L2274-L2296)).
- **Memory limits.** `-m` caps memory through cgroups, with no delegation needed ([resource_constraints.md L85](https://github.com/docker/docs/blob/226023fc89bdeb3f051c4c8c697d04ed5f5bcd2c/content/manuals/engine/containers/resource_constraints.md#L85)).

**Rootless**
- **How it runs.**
  - The daemon runs as jacob in a user namespace ([rootless/_index.md L17-L37](https://github.com/docker/docs/blob/226023fc89bdeb3f051c4c8c697d04ed5f5bcd2c/content/manuals/engine/security/rootless/_index.md#L17-L37)).
  - It runs as a `systemctl --user` service, which needs `loginctl enable-linger` to keep running without a login ([L72-L80](https://github.com/docker/docs/blob/226023fc89bdeb3f051c4c8c697d04ed5f5bcd2c/content/manuals/engine/security/rootless/_index.md#L72-L80)).
- **Setup here.** On Ubuntu 24.04, the rootlesskit profile from the apt package suffices ([troubleshoot.md L12-L44](https://github.com/docker/docs/blob/226023fc89bdeb3f051c4c8c697d04ed5f5bcd2c/content/manuals/engine/security/rootless/troubleshoot.md#L12-L44)).
- **Host loopback is off by default.** `DOCKERD_ROOTLESS_ROOTLESSKIT_DISABLE_HOST_LOOPBACK` "Defaults to true" ([dockerd-rootless.sh L13-L24](https://github.com/moby/moby/blob/0367cbeb49b9d98cb0b4f51e95f7b54b4803e76c/contrib/dockerd-rootless.sh#L13-L24)).
- **Limits.** cgroup limits work "only when running with cgroup v2 and systemd" ([troubleshoot.md L72](https://github.com/docker/docs/blob/226023fc89bdeb3f051c4c8c697d04ed5f5bcd2c/content/manuals/engine/security/rootless/troubleshoot.md#L72)).

**Both**
- **Overhead.** dockerd and containerd stay resident. No primary figures were found **[unverified]**.

### 3.5 systemd-nspawn

- **Root.**
  - v255 refuses to run as an ordinary user: "Need to be root." ([nspawn.c L5396-L5399](https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/src/nspawn/nspawn.c#L5396-L5399)).
  - Unprivileged operation from a DDI image arrived in v256 ([NEWS L6210-L6236](https://github.com/systemd/systemd/blob/73612ea502d53baf9b0df4336cfc296a7ee0d524/NEWS#L6210-L6236)). Ubuntu 24.04 has 255.
- **Network.**
  - `--private-network` leaves only loopback plus assigned interfaces. Without it, a container has "full access to the host network".
  - `--network-veth` relies on systemd-networkd for DHCP and NAT ([systemd-nspawn.xml L891-L990](https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/man/systemd-nspawn.xml#L891-L990)).
- **Limits.** `--property=` "is useful to set memory limits" ([L711-L716](https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/man/systemd-nspawn.xml#L711-L716)).
- **Fit [interpretation].** It needs root at every start, or a root-run unit, for nothing bwrap doesn't already do.

### 3.6 gVisor

- **Network.**
  - `runsc do` is "to be used for testing only" ([do.go L68-L82](https://github.com/google/gvisor/blob/60faa657ca99f3398e04c8dbc580a848df20f2f2/runsc/cmd/do.go#L68-L82)). As root it builds a veth pair and NAT.
  - With `--rootless`, it prints "sandbox network isn't supported with --rootless, switching to host" ([L254-L258](https://github.com/google/gvisor/blob/60faa657ca99f3398e04c8dbc580a848df20f2f2/runsc/cmd/do.go#L254-L258)). Rootless mode doesn't support gVisor's own network stack ([rootless.md L36-L53](https://github.com/google/gvisor/blob/60faa657ca99f3398e04c8dbc580a848df20f2f2/g3doc/user_guide/rootless.md#L36-L53)).
  - So rootless gVisor offers either the host's network, loopback included, or none ([networking.md L23-L48](https://github.com/google/gvisor/blob/60faa657ca99f3398e04c8dbc580a848df20f2f2/g3doc/user_guide/networking.md#L23-L48)) **[interpretation]**.
- **Cost.**
  - It intercepts every syscall. systrap is the default platform, and inside a VM, nested KVM gives "poor performance" ([platforms.md L14-L45](https://github.com/google/gvisor/blob/60faa657ca99f3398e04c8dbc580a848df20f2f2/g3doc/user_guide/platforms.md#L14-L45)).
  - Its published figures, from old benchmark output, put an empty container at 23.7 MB against runc's 4.1 MB ([density.csv L2-L5](https://github.com/google/gvisor/blob/60faa657ca99f3398e04c8dbc580a848df20f2f2/website/performance/density.csv#L2-L5)).
- **Compatibility and limits.** io_uring is off ([compatibility.md L65-L66](https://github.com/google/gvisor/blob/60faa657ca99f3398e04c8dbc580a848df20f2f2/g3doc/user_guide/compatibility.md#L65-L66)). Limits must come from a host cgroup ([L46-L50](https://github.com/google/gvisor/blob/60faa657ca99f3398e04c8dbc580a848df20f2f2/g3doc/user_guide/compatibility.md#L46-L50)).
- **Setup.** `--rootless` creates a user namespace, so runsc would need an AppArmor profile of its own **[interpretation]**.
- **Fit [interpretation].** It has the strongest syscall boundary of the container options. But it has no rootless network that reaches a provider without also reaching the host's loopback, and it taxes every git command and build.

### 3.7 Firecracker and other microVMs

- **It is possible here, barely.**
  - It needs read-write `/dev/kvm` ([getting-started.md L27-L48](https://github.com/firecracker-microvm/firecracker/blob/95f868c8e345b1cc8faccd1a3c910b4989dc3f58/docs/getting-started.md#L27-L48)). The host has it, nested, but jacob isn't in group `kvm`.
  - "We run the jailer as the root user" ([jailer.md L291-L292](https://github.com/firecracker-microvm/firecracker/blob/95f868c8e345b1cc8faccd1a3c910b4989dc3f58/docs/jailer.md#L291-L292)).
  - Its networking uses `sudo ip tuntap` and NAT rules ([network-setup.md L57-L109](https://github.com/firecracker-microvm/firecracker/blob/95f868c8e345b1cc8faccd1a3c910b4989dc3f58/docs/network-setup.md#L57-L109)).
  - Its validated host kernels don't include 6.8 ([kernel-policy.md L24-L30](https://github.com/firecracker-microvm/firecracker/blob/95f868c8e345b1cc8faccd1a3c910b4989dc3f58/docs/kernel-policy.md#L24-L30)).
- **There is no shared filesystem.**
  - Its devices are virtio net, block and vsock ([design.md L74-L77](https://github.com/firecracker-microvm/firecracker/blob/95f868c8e345b1cc8faccd1a3c910b4989dc3f58/docs/design.md#L74-L77)).
  - virtio-fs "is not currently on our roadmap" ([#1180](https://github.com/firecracker-microvm/firecracker/issues/1180#issuecomment-1785379904)).
  - So a login refreshed inside the VM sits on a block device until something copies it out **[interpretation]**.
- **Overhead.** The VMM itself takes "<= 5 MiB", and boot reaches `/sbin/init` in "<= 125 ms" ([SPECIFICATION.md L8-L42](https://github.com/firecracker-microvm/firecracker/blob/95f868c8e345b1cc8faccd1a3c910b4989dc3f58/SPECIFICATION.md#L8-L42)). Each guest's own RAM comes on top.
- **The alternatives with virtio-fs still need privilege.**
  - Cloud Hypervisor's example gives virtiofsd `cap_sys_admin` ([fs.md L6-L22](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/038ce5a72d761380a02f714a4eb4ca63e0ea073d/docs/fs.md#L6-L22)).
  - Kata defaults to 2,048 MiB per VM ([virtio-fs how-to L7-L18](https://github.com/kata-containers/kata-containers/blob/bd75006ebc9dea6fcb04d8e5ae4e40a3f9316fbf/docs/how-to/how-to-use-virtio-fs-with-kata.md#L7-L18)). Its shim and virtiofsd "still run as root" ([rootless-vmm L9-L10](https://github.com/kata-containers/kata-containers/blob/bd75006ebc9dea6fcb04d8e5ae4e40a3f9316fbf/docs/how-to/how-to-run-rootless-vmm.md#L9-L10)).
- **Fit [interpretation].** It has the strongest boundary. The cost is root plumbing, a guest image holding the Harnesses, gigabytes of guest RAM on an 8 GB host, and no simple path for logins.

### 3.8 The Harnesses' own sandboxes

- **Claude Code** sandboxes shell commands only.
  - "Claude's file tools, MCP servers, and hooks run outside it", and "A `denyRead` entry doesn't stop the Read tool" ([sandboxing](https://code.claude.com/docs/en/sandboxing#what-runs-outside-the-sandbox)).
  - By default, reads cover "Most of the machine, including credential files".
  - Anthropic says the sandboxed Bash tool "is not sufficient for fully unattended runs" ([sandbox-environments](https://code.claude.com/docs/en/sandbox-environments)).
- **Codex** confines the commands it spawns.
  - workspace-write "can read the entire disk" ([protocol.rs L1316-L1330](https://github.com/openai/codex/blob/83086188ae0614bd882818ff1aa23f3194962012/codex-rs/protocol/src/protocol.rs#L1316-L1330)).
  - Its proxy doesn't filter "the client's model and authentication requests" ([agent-approvals-security](https://learn.chatgpt.com/docs/agent-approvals-security#traffic-outside-the-command-network-proxy)).
  - It now prefers "the first `bwrap` found on `PATH`", and falls back to a bundled copy ([linux-sandbox README L16-L23](https://github.com/openai/codex/blob/83086188ae0614bd882818ff1aa23f3194962012/codex-rs/linux-sandbox/README.md#L16-L23)). So on this host it would use `/usr/bin/bwrap` and its profile **[interpretation; needs live check]**. That reopens ADR 0012's premise that Codex's sandbox can't start here.
- **Grok Build** applies Landlock to "the agent process and its children", and it is off by default ([sandbox](https://docs.x.ai/build/features/sandbox)).
- **Muse Code** uses "a bundled bubblewrap helper" ([permissions](https://dev.meta.ai/docs/muse-code/permissions)). The host's profile is attached to `/usr/bin/bwrap`, so it likely doesn't cover that helper **[interpretation]**.
- **agy** uses "`nsjail` on Linux", and its sandbox is off by default ([CLI features](https://antigravity.google/docs/cli/features); [settings](https://antigravity.google/docs/settings)).
- **OpenCode** has none: no bwrap, Landlock or seccomp code was found in v1.19.3's source.
- **Fit [interpretation].** These are six different mechanisms. Most leave the disk readable and the Harness process itself unconfined, and ADR 0013 turns all of them off in production. Turning them on in Trials would make Trials differ from Runs in behaviour, not only in what they can see.

### 3.9 Side by side

"Leaks" below means host loopback, the host's public addresses, and GitHub.

| Option | Files | Leaks | Processes | Memory limits | Setup on this host | Root or daemon | Logins |
|---|---|---|---|---|---|---|---|
| bwrap as Q3.6 settled it | Only what is bound | All three open | Host command lines visible | None | Done | None | Bind the file |
| bwrap + PID namespace + pasta | Only what is bound | Loopback closed; public addresses and GitHub open | Own namespace; orphans die with it | Through a systemd scope (§7.2) | `apt install passt`; pasta profile **[needs live check]** | None | Bind the file |
| bwrap + PID namespace + slirp4netns | Only what is bound | As pasta; DNS reaches the host's stub | As above | As above | `apt install slirp4netns`; profile shipped | None | Bind the file |
| bwrap + PID namespace + allowlisting proxy | Only what is bound | All closed | As above | As above | socat is installed; a proxy to write; each Harness must honour `HTTPS_PROXY` | None | Bind the file |
| Rootless Podman | Image plus binds | Loopback closed by default | Own namespace | Only with a user session (lingering) | `apt install podman uidmap slirp4netns`; an image per repository | No daemon | `keep-id` binds |
| Docker, rootful | Image plus binds | Loopback closed on a bridge; host's public addresses open | Own namespace | Yes | Install Docker; an image per repository | Root daemon; docker group is root | Bind, watching ownership |
| Docker, rootless | Image plus binds | Loopback closed by default | Own namespace | Only with systemd and lingering | Install; user service | User daemon | `keep-id`-style binds |
| systemd-nspawn | Directory or image | Closed with `--private-network`, but then no way out | Own namespace | Yes, `--property=` | Install `systemd-container` | Root at every start | Bind |
| gVisor, rootless | Overlay or binds | Host network or none | Own kernel | From a host cgroup | Install runsc; AppArmor profile | None | Bind |
| Firecracker | Block-device images | Closed through TAP and NAT | Own kernel | Guest RAM | kvm group; root for jailer and TAP | Root setup | Copy in and out |
| Harness sandboxes | Mostly whole disk readable | Varies | Commands only | None | Varies; Muse's is blocked | None | Unchanged |

## 4. Logins inside a sandbox

| Harness | Where the login is | How it is written | In a sandbox |
|---|---|---|---|
| Claude Code | `~/.claude/.credentials.json`, mode 0600, or under `CLAUDE_CONFIG_DIR` ([authentication](https://code.claude.com/docs/en/authentication#credential-management)) | Not documented **[unverified]** | Better to skip the file (see below). Binding it works only if Claude writes it in place **[needs live check]** |
| Codex | `$CODEX_HOME/auth.json` | In place: `truncate(true).write(true).create(true)`, mode 0600, with no rename ([storage.rs L214-L231](https://github.com/openai/codex/blob/83086188ae0614bd882818ff1aa23f3194962012/codex-rs/login/src/auth/storage.rs#L214-L231)) | Bind the one real file read-write |
| OpenCode | `$XDG_DATA_HOME/opencode/auth.json`, holding an API key ([auth/index.ts L10-L88](https://github.com/anomalyco/opencode/blob/e3c4eb242585dcd77be1a867391b7c25fd47ce52/packages/opencode/src/auth/index.ts#L10-L88)) | In place, then chmod 0600 | Bind it read-only, or pass `OPENCODE_AUTH_CONTENT` |
| Grok Build | `GROK_HOME`, by default `~/.grok`, is the "Home for config, auth, sessions, skills, plugins, and logs" ([settings reference](https://docs.x.ai/build/settings/reference)). Logins go to `auth.json` ([enterprise](https://docs.x.ai/build/enterprise)) | **[unverified]** | Bind only the auth file **[needs live check]** |
| Muse Code | "Muse Code stores the credential for you"; where is not said ([auth](https://dev.meta.ai/docs/muse-code/auth)) | **[unverified]** | **[needs live check]** |
| agy | The "native secure keyring", through Secret Service over D-Bus ([install](https://antigravity.google/docs/install)) | **[unverified]** | Under `env -i`, the keyring lookup failed and agy fell back to its token file ([harness-candidates.md §1](harness-candidates.md#1-agy-antigravity-cli-1217-gemini-38-flash)). Bind that file, and never the D-Bus session bus, which would expose every user service on it **[interpretation; needs live check]** |

**Rules that follow [interpretation].**
- **Bind files, not homes.** Each Harness home holds more than its login:
  - Grok's holds sessions and logs;
  - `CLAUDE_CONFIG_DIR` holds "All settings, session history, and plugins" ([env-vars](https://code.claude.com/docs/en/env-vars));
  - Codex keeps every session's rollout under `~/.codex/sessions/` (the grilling's 2026-10-21 results).

  Those histories include Graders' sessions, and the production sessions that wrote the later fixes.
- **A fresh home per Trial turns memory off by construction.**
  - Only the login, and the user-level instructions, rules, settings and hooks a Run's session reads, are bound in, read-only.
  - The documented switches can make it explicit: `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1` ([memory](https://code.claude.com/docs/en/memory#enable-or-disable-auto-memory)) and `GROK_MEMORY=0` ([settings reference](https://docs.x.ai/build/settings/reference)). Muse's switch is **[unverified]**.
  - Binding the user-level files keeps a Trial told what a Run is told **[needs live check per Harness]**.
- **Bind the one real file, in place, and never a copy.**
  - Codex refreshes within five minutes of its access token's expiry, or, if it can't read the expiry, eight days after the last refresh.
  - A refresh returns a new refresh token, and reusing an old one fails with "your refresh token was already used" ([manager.rs L203-L214](https://github.com/openai/codex/blob/83086188ae0614bd882818ff1aa23f3194962012/codex-rs/login/src/auth/manager.rs#L203-L214), [L1599-L1623](https://github.com/openai/codex/blob/83086188ae0614bd882818ff1aa23f3194962012/codex-rs/login/src/auth/manager.rs#L1599-L1623), [L1686-L1690](https://github.com/openai/codex/blob/83086188ae0614bd882818ff1aa23f3194962012/codex-rs/login/src/auth/manager.rs#L1686-L1690)).
  - So a copy that refreshes leaves the factory's file holding a used token.
- **A single-file bind survives in-place writes, but not a rename.** The kernel refuses to rename onto a mount point: `error = -EBUSY; if (is_local_mountpoint(old_dentry) || is_local_mountpoint(new_dentry))` ([namei.c L4861-L4863](https://github.com/torvalds/linux/blob/e8f897f4afef0031fe618a8e94127a0934896aba/fs/namei.c#L4861-L4863)).
  - So a Harness that saves its login by writing a temporary file and renaming it can't save through a single-file bind.
  - Binding its directory instead exposes whatever else is in that directory.
- **Concurrency is already production's risk.**
  - Codex's docs say "Use one `auth.json` per runner or per serialized workflow stream. Do not share the same file across concurrent jobs or multiple machines" ([ci-cd-auth](https://learn.chatgpt.com/docs/auth/ci-cd-auth)).
  - The factory already shares the file between about two concurrent Runs, and two Trials would add two more users.
  - Codex rereads the file before refreshing on a 401 ([manager.rs L1850-L1858](https://github.com/openai/codex/blob/83086188ae0614bd882818ff1aa23f3194962012/codex-rs/login/src/auth/manager.rs#L1850-L1858)), which is likely what keeps this working **[needs live check]**.
- **Claude needs no file.**
  - `claude setup-token` makes a "one-year OAuth token" for `CLAUDE_CODE_OAUTH_TOKEN`. It "authenticates with your Claude subscription", for "CI pipelines, scripts, or other environments where interactive browser login isn't available".
  - It "can only make model requests", so no Remote Control and no claude.ai connectors ([authentication](https://code.claude.com/docs/en/authentication#generate-a-long-lived-token)). A Trial uses neither **[interpretation]**.
  - With it, there is nothing to bind and nothing to refresh.
- **In containers, the same rules apply, plus ownership.** The file must stay jacob's, which rootless Podman's `keep-id` keeps (§3.3).

## 5. Toolchains for the implement suite

### 5.1 How the benchmarks build a historical commit

- **SWE-bench 5.0.2.**
  - **One Dockerfile per instance.** Each instance has a generated Dockerfile in a task repository, built with buildx ([cli.md L46-L82](https://github.com/SWE-bench/SWE-bench/blob/490635b2e9e775dca1e1d6b40ce9dbcff91e780f/docs/reference/cli.md#L46-L82)). It concatenates three layers:
    - Ubuntu with Miniconda;
    - a conda environment from pinned specifications;
    - the repository ([constants.py L1-L33](https://github.com/SWE-bench/swe-bench-tasks/blob/3d07b464b7b311a0cbfb5ed5b2d8a3b96f84a33d/dockerfile_gen/constants.py#L1-L33)).
  - **Rust tasks** start `FROM rust:1.81` ([ripgrep Dockerfile L2](https://github.com/SWE-bench/swe-bench-multilingual-tasks/blob/6e08cbcd763c869abd7aae4bcb9e1b3991180777/tasks/burntsushi__ripgrep-2576/Dockerfile#L2)).
  - **The clone is time-safe** ([django-11099 Dockerfile L114-L126](https://github.com/SWE-bench/swe-bench-tasks/blob/3d07b464b7b311a0cbfb5ed5b2d8a3b96f84a33d/tasks/django__django-11099/Dockerfile#L114-L126)). It:
    - resets to the base commit and removes `origin`;
    - deletes later tags and other branches;
    - expires the reflog and runs `git gc --prune=now`;
    - fails the build if `git log --all` shows any commit after the base.

    That is the check Q3.6 wants for `replay.py`.
- **SWE-bench's sizes.**
  - Its README asks for "at least 120GB of free storage, 16GB of RAM, and 8 CPU cores" ([README L110-L117](https://github.com/SWE-bench/SWE-bench/blob/490635b2e9e775dca1e1d6b40ce9dbcff91e780f/README.md#L110-L117)).
  - Its docs, still describing v4's `cache_level`, put storage at about 120 GB, 100 GB or 2,000 GB depending on how much is cached ([harness.md L115-L126](https://github.com/SWE-bench/SWE-bench/blob/490635b2e9e775dca1e1d6b40ce9dbcff91e780f/docs/reference/harness.md#L115-L126)).
  - Its published instance images are 0.27–1.4 GB each, compressed ([Docker Hub API](https://hub.docker.com/v2/repositories/swebench/sweb.eval.x86_64.django_1776_django-11099/tags/?page_size=10)). Docker reports their larger, uncompressed size ([What is an image?](https://docs.docker.com/get-started/docker-concepts/the-basics/what-is-an-image/)).
- **Harbor.**
  - A task names either a prebuilt `docker_image` or an `environment/Dockerfile`, built with `docker compose build` ([definition.py L26-L36](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/environments/definition.py#L26-L36)).
  - Its SWE-bench adapter starts `FROM` SWE-bench's published image ([Dockerfile template L18-L31](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/adapters/swebench/src/swebench_adapter/task-template/environment/Dockerfile#L18-L31)). That image's `:latest` tag is mutable.
- **Terminal-Bench 2.0.**
  - 89 tasks. Each has a Dockerfile and a prebuilt `alexgshaw/<task>:20251031` image ([task.toml L27-L35](https://github.com/harbor-framework/terminal-bench-2/blob/2fd12b88aafdd04a52c298e3940bcb189f9766d6/adaptive-rejection-sampler/task.toml#L27-L35)).
  - The images total about 49 GB, with a median of about 149 MB and two near 8.8 GB (Docker Hub metadata, summed by the Harbor read-through).
  - 84 tasks ask for 1 CPU, and 69 for 2 GB.
  - All allow internet, and every verifier downloads uv and pytest ([test.sh L1-L31](https://github.com/harbor-framework/terminal-bench-2/blob/2fd12b88aafdd04a52c298e3940bcb189f9766d6/adaptive-rejection-sampler/tests/test.sh#L1-L31)).

### 5.2 Per-repository images on this host [interpretation]

- **Disk.**
  - Seven repositories use four toolchains. Compressed base images ([Docker Hub API](https://hub.docker.com/v2/repositories/library/rust/tags/1.99.1-bookworm)):
    - `rust:1.99.1-bookworm` 570 MB (slim 321 MB);
    - `node:24.22.1-bookworm` 410 MB (slim 81 MB);
    - `php:8.3.35-cli-bookworm` 181 MB;
    - `python:3.12.15-bookworm` 380 MB (slim 45 MB).
  - On disk they take several times that. Each image also needs the Harness CLIs, dependencies, and build output per Trial (thirdshift's is 1.6–2.7 GB per worktree, §1).
  - 32 GB free holds only a few such images, beside the factory's 16 GB of checkouts and caches.
- **Upkeep.**
  - A Dockerfile per repository, rebuilt when its toolchain pin moves. thirdshift's moves by pull request.
  - The six Harness CLIs installed in each image at the versions production runs. Otherwise Trials stop matching Runs, and an Arm is supposed to differ only by its build, Harness, Model and Effort (Q3.8).
- **Gains.**
  - Old commits still build after the host has moved on.
  - Repositories that need services, such as keeplore with its compose file, get them.

### 5.3 The host's toolchains [interpretation]

- **The suite's Eval tasks are recent merged fixes**, weeks old, in repositories this host builds every day.
  - rustup installs whatever `rust-toolchain.toml` pins at each commit.
  - Node, Python and PHP each have one version.
- **Prepare each Eval task once, before its Trials.**
  - Build and test at the base commit, with network, in the same bwrap.
  - Check that the merged fix's tests fail there and pass at the fix.
  - Keep the result read-only. Each Trial then copies it: bwrap 0.9.0 has no overlay mounts (§3.1), and ext4 has no reflinks (§1).
  - Two thirdshift Trials at 1.6–2.7 GB each fit in 32 GB.
- **Code Graders** run the merged fix's tests offline, as SWE-bench's `eval.sh` does ([eval.sh L14-L46](https://github.com/SWE-bench/swe-bench-tasks/blob/3d07b464b7b311a0cbfb5ed5b2d8a3b96f84a33d/tasks/django__django-11099/eval.sh#L14-L46)). They run in bwrap with `--unshare-net` and the Security skill's limits (§2).

## 6. What Docker would open up

### 6.1 Harbor, Terminal-Bench and the Phoenix plugin

- **Licence and health.**
  - Harbor is Apache-2.0, not MIT as the talk said ([LICENSE L1-L2](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/LICENSE#L1-L2)).
  - It needs Python 3.12 or later and has 23 direct dependencies ([pyproject.toml L9-L34](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/pyproject.toml#L9-L34)), about 86 packages in all by the read-through's count.
  - It shipped four minor releases in 90 days, plus nightlies.
- **Environments.**
  - It knows 34 environment types. The local ones are docker, podman, kata, singularity, apple-container, sbx and smol ([environment_type.py L4-L38](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/models/environment_type.py#L4-L38)).
  - Docker runs through `docker compose`, and the daemon must be running ([docker.py L173-L191](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/environments/docker/docker.py#L173-L191)).
  - Podman reuses that code and needs "`systemctl --user enable --now podman.socket`" ([podman.py L95-L99](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/environments/podman.py#L95-L99)). That means a user manager, and so lingering on this host **[interpretation]**.
  - CPU and memory are hard compose limits by default ([docker.py L555-L568](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/environments/docker/docker.py#L555-L568)).
  - Network modes are `public` (the default), `no-network` and `allowlist` ([config.py L40-L45](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/models/task/config.py#L40-L45), [L250-L256](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/models/task/config.py#L250-L256)). The restricted modes use a gost proxy sidecar with nftables and `NET_ADMIN` ([egress-control compose](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/environments/docker/docker-compose-egress-control.yaml#L1-L16)).
- **Built-in agents and their logins.** They start sessions their own way, not through thirdshift.
  - **Claude Code** takes `CLAUDE_CODE_OAUTH_TOKEN`, but only with `CLAUDE_FORCE_OAUTH` set: "CLAUDE_FORCE_OAUTH drops the API key so the CLI uses the subscription token" ([claude_code.py L2009-L2060](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/agents/installed/claude_code.py#L2009-L2060)).
  - **Codex** uploads a copy of `auth.json`, links it into `CODEX_HOME`, and deletes both after the run without copying anything back ([codex.py L1975-L1994](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/agents/installed/codex.py#L1975-L1994), [L2093-L2103](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/agents/installed/codex.py#L2093-L2103)). That is the risky copy of §4.
  - **Grok Build** ("Auth requires the `XAI_API_KEY`", [grok_build.py L208-L209](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/agents/installed/grok_build.py#L208-L209)), **Muse Code** (`META_API_KEY`, [muse_code.py L91-L102](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/agents/installed/muse_code.py#L91-L102)), **agy** (a Gemini key or ADC, [antigravity_cli.py L340-L418](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/agents/installed/antigravity_cli.py#L340-L418)) and **OpenCode** (a provider key, [opencode.py L305-L318](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/agents/installed/opencode.py#L305-L318)) need API keys.
  - Credentials travel as `docker compose exec -e KEY=VALUE` arguments ([docker.py L1418-L1420](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/environments/docker/docker.py#L1418-L1420)), so they show in the host's process list **[interpretation]**.
- **A custom agent could keep Trials on thirdshift's code [interpretation].**
  - `BaseInstalledAgent` runs inside the environment. `BaseAgent` runs on the host and acts through `environment.exec`. Both can upload files ([custom-agents.mdx L10-L19](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/docs-mintlify/agents/custom-agents.mdx#L10-L19)).
  - agy's adapter already uploads a local binary.
  - So an agent could upload the thirdshift build, the Harness and a login, run the hidden flag once, and leave its log in `agent/`.
  - Nobody documents that pattern. The binaries must match the image's libc, and §4's login problems apply inside the container.
- **`infra_ok` isn't Harbor's.** Harbor's source has no `infra_ok`.
  - **It is a score in Phoenix's plugin:** "`1` when Harbor records no trial or step exception, otherwise `0`", used "to separate agent behavior from broken environments, timeouts, or verifier failures". The reward is kept ([harbor.mdx L19, L206-L214](https://github.com/Arize-ai/phoenix/blob/f6a023154da2c899e6937d49b37b2ebce8e8609b/docs/phoenix/integrations/evaluation-integrations/harbor.mdx#L206-L214); [_scores.py L125-L139](https://github.com/Arize-ai/phoenix/blob/f6a023154da2c899e6937d49b37b2ebce8e8609b/packages/phoenix-client/src/phoenix/client/harbor/_scores.py#L125-L139)). So a timeout makes it 0 as well.
  - **Harbor's `main` adds an error policy** that v0.24.0 doesn't have ([error_policy.py L14-L138](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/models/trial/error_policy.py#L14-L138)). It names failure causes:
    - `sandbox_failed`, which covers environment start timeouts, build failures, health checks and out-of-memory, and is retried but not graded;
    - `model_provider_failed`, `verifier_failed` and `cancelled`;
    - the agent's own causes.

    A reward counts as valid only for `agent_exhausted_budget`, which includes `AgentTimeoutError`, and `agent_refused_task`. Only `result.py` uses the policy so far.
  - **Harbor's metrics score a trial with no verifier result as 0** ([metrics/base.py L23-L34](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/metrics/base.py#L23-L34)).
- **The Phoenix plugin.**
  - It lives in Phoenix's client (`arize-phoenix-client[harbor]`, [pyproject.toml L46-L55](https://github.com/Arize-ai/phoenix/blob/f6a023154da2c899e6937d49b37b2ebce8e8609b/packages/phoenix-client/pyproject.toml#L46-L55)) and runs live during a job.
  - It needs a Phoenix server, version 15.0 or later, by default at `http://localhost:6006` ([harbor.mdx L53-L72](https://github.com/Arize-ai/phoenix/blob/f6a023154da2c899e6937d49b37b2ebce8e8609b/docs/phoenix/integrations/evaluation-integrations/harbor.mdx#L53-L72)).
  - It sends the tasks, one experiment per agent and Model, each run's final text, rewards, `infra_ok` and exception, and its ATIF trajectory as OpenInference spans.
  - The Phoenix server is ELv2 ([LICENSE L1](https://github.com/Arize-ai/phoenix/blob/f6a023154da2c899e6937d49b37b2ebce8e8609b/LICENSE#L1)).
- **Egress.**
  - Harbor's telemetry goes to PostHog at `us.i.posthog.com`. It is on unless `HARBOR_TELEMETRY` is set to `off` or similar ([telemetry.py L40-L45](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/telemetry.py#L40-L45), [L479-L480](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/src/harbor/telemetry.py#L479-L480)).
  - LiteLLM, which Harbor imports, fetches its price map from GitHub unless `LITELLM_LOCAL_MODEL_COST_MAP=True` ([litellm `__init__.py` L394-L397](https://github.com/BerriAI/litellm/blob/052b5a2169d8d3082e1d66e69f200a72b0c1e274/litellm/__init__.py#L394-L397)).
- **Terminal-Bench 2.0** is in Harbor's registry as `terminal-bench@2.0` ([registry.json L94012-L94019](https://github.com/harbor-framework/harbor/blob/6c9d3d6609854375005a7b5c8f1906371d64c6ff/registry.json#L94012-L94019)). Its sizes are in §5.1. It tests a Harness and Model on public tasks, not thirdshift's skills or prompts **[interpretation]**.

### 6.2 Inspect's Docker sandbox and inspect_swe

- **What the sandbox does.**
  - Each sample gets its own sandbox ([sandboxing.qmd L185](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/sandboxing.qmd#L185)).
  - "The generated Compose configuration sets `network_mode: none`". A custom configuration replaces it, and "Docker Compose's default is a project-scoped network with outbound Internet access" ([L223-L227](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/sandboxing.qmd#L223-L227)).
  - It sets no CPU or memory limits unless given ([L461-L474](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/sandboxing.qmd#L461-L474)).
  - It needs Docker Engine 24.0.6 and Compose 2.21.0 or later ([L201-L205](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/sandboxing.qmd#L201-L205)). Podman appears only as a hypothetical extension ([extensions-sandboxes.qmd L43-L44](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/extensions-sandboxes.qmd#L43-L44)).
  - A solver can `exec()` a binary inside the sandbox with a timeout, and `write_file()` into it ([docker.py L319-L406](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/src/inspect_ai/util/_sandbox/docker/docker.py#L319-L406)).
  - A sandbox that fails to start is a sample error. `retry_on_error` defaults to 0 ([handling-errors.qmd L46-L119](https://github.com/UKGovernmentBEIS/inspect_ai/blob/386e09942510e93de2db6d108f24f62f65eecbdc/docs/handling-errors.qmd#L46-L119)).
- **inspect_swe has no subscription path.**
  - "Model API calls that occur in the sandbox are proxied back to Inspect for handling by the model provider" ([agent_intro.md L3](https://github.com/meridianlabs-ai/inspect_swe/blob/377b1112ef7177190844100825baba64706536bc/docs/_snippets/agent_intro.md#L3)).
  - For Claude Code it writes an `apiKeyHelper`, because otherwise Claude Code "enters an OAuth flow that silently fails" ([claude_code.py L445-L451](https://github.com/meridianlabs-ai/inspect_swe/blob/377b1112ef7177190844100825baba64706536bc/src/inspect_swe/_claude_code/claude_code.py#L445-L451)).
  - For Codex it sets `preferred_auth_method = "apikey"` ([codex_cli.py L650-L671](https://github.com/meridianlabs-ai/inspect_swe/blob/377b1112ef7177190844100825baba64706536bc/src/inspect_swe/_codex_cli/codex_cli.py#L650-L671)).
- **So Docker would give Inspect a container per Trial [interpretation].** The Trial would still need thirdshift's flag, a Harness and a login inside that container, and a network that reaches the provider, which means replacing the generated `network_mode: none`.

### 6.3 The SWE-bench harness

- **What it grades.** Patches.
  - It applies the model's patch, falling back through `git apply --3way`, `--reject` and `patch --fuzz=5`. Then it runs `eval.sh` with the test patch.
  - An instance is resolved when every FAIL_TO_PASS and PASS_TO_PASS test passes ([grading.py L309-L326](https://github.com/SWE-bench/SWE-bench/blob/490635b2e9e775dca1e1d6b40ce9dbcff91e780f/swebench/harness/grading.py#L309-L326)).
- **Its containers** get Docker's default network and `cap_add=["SYS_ADMIN"]` ([run_evaluation.py L116-L125](https://github.com/SWE-bench/SWE-bench/blob/490635b2e9e775dca1e1d6b40ce9dbcff91e780f/swebench/harness/run_evaluation.py#L116-L125)).
- **Errors.**
  - A patch that won't apply, a container error and a timeout all end as `error` ([L414-L429](https://github.com/SWE-bench/SWE-bench/blob/490635b2e9e775dca1e1d6b40ce9dbcff91e780f/swebench/harness/run_evaluation.py#L414-L429)).
  - Likely infrastructure failures are listed in `infra_failure_ids`, but they "stay in unresolved_ids/error_ids, so the denominator is unchanged" ([reporting.py L36-L109](https://github.com/SWE-bench/SWE-bench/blob/490635b2e9e775dca1e1d6b40ce9dbcff91e780f/swebench/harness/reporting.py#L36-L109)).
- **New repositories** need their specifications in a task repository. A new test runner needs a log parser in SWE-bench's own code ([grading.py L125](https://github.com/SWE-bench/SWE-bench/blob/490635b2e9e775dca1e1d6b40ce9dbcff91e780f/swebench/harness/grading.py#L125)).
- **SWE-bench Multilingual** covers "C, Go, Java, JavaScript, PHP, Ruby, Rust" ([README L3](https://github.com/SWE-bench/swe-bench-multilingual-tasks/blob/6e08cbcd763c869abd7aae4bcb9e1b3991180777/README.md#L3)).
- **Fit [interpretation].** It is a reference to copy (the history scrub, the output markers, the exit-code cross-check), not a dependency.

### 6.4 Langfuse on `stephens`

- **The docs haven't changed.** They still say "at least 4 cores and 16 GiB of memory", and storage of "e.g. 100GiB" ([docker-compose.mdx L95-L97](https://github.com/langfuse/langfuse-docs/blob/5ee3f4e54283dedf26780ac6f38072012b2c2d88/content/self-hosting/deployment/docker-compose.mdx#L95-L97)). The only commit since `b0e8f96` touched blog files.
- **The Helm minimums total 25.5 GiB** ([scaling.mdx L12-L21](https://github.com/langfuse/langfuse-docs/blob/5ee3f4e54283dedf26780ac6f38072012b2c2d88/content/self-hosting/configuration/scaling.mdx#L12-L21)):
  - web 4 GiB, worker 4 GiB and Postgres 4 GiB;
  - Redis 1.5 GiB;
  - ClickHouse 8 GiB;
  - MinIO 4 GiB.
- **No low-resource configuration is documented.**
- **ClickHouse's own guidance** ([tips.mdx L41-L59](https://github.com/ClickHouse/ClickHouse/blob/1b29cdec0333ca779336d4a688b176644b80bcc1/docs/guides/oss/best-practices/tips.mdx#L41-L59)):
  - "The recommended amount of RAM is 32 GB or more."
  - Below 16 GB, "you may experience various memory exceptions".
  - It runs on as little as 2 GB with tuning, but "can only ingest at a low rate".
- **`stephens`** has 15,993 MiB with 2.7 GiB of swap in use, and it runs factory Runs. It already has Docker, which was never the obstacle **[interpretation]**.

## 7. How each option changes the decisions

### 7.1 Q3.6 and the loopback risk

- **(a) stands, and bwrap stays the boundary [interpretation].** It needs three additions:
  - `--unshare-pid`, with a fresh `/proc` and `--die-with-parent`. This covers the command-line leak and the orphans (§2).
  - `--unshare-net`, with a way out to the provider: the allowlisting proxy, or pasta with the loopback mapping off (§3.2).
  - A fresh home per Trial, with only the login and the user-level files a Run's session reads bound in. This turns Harness memory off by construction (§4).

  `--unshare-ipc` and `--unshare-uts` cost nothing more.
- **The Settled list's condition is now known.** "If a Trial can reach loopback, each Trial gets a network of its own." It can: bwrap shares the host's network, and the Console and Caddy's admin API listen there (§1). So every Trial gets its own network, with no live check needed to decide that.
- **The later-commit check** can follow SWE-bench's time-safe clone (§5.1).
- **The live checks change.** They now cover:
  - each Harness's login inside bwrap, per §4;
  - each Harness behind `HTTPS_PROXY`, or pasta's attach mode on 24.04;
  - the user-level files each Harness needs.
- **Graders run the same way.** Model Graders run in the same sandbox, seeing only their packet. Code Graders run with no network at all.

### 7.2 Q3.7: protecting the factory's memory

- **Yes, cgroups could protect it, but not from cron as the host stands.**
  - A process in root-owned `cron.service` can't create sub-cgroups. Delegation means "granting write access of the directory and its cgroup.procs" ([cgroup-v2.rst L517-L564](https://github.com/torvalds/linux/blob/e8f897f4afef0031fe618a8e94127a0934896aba/Documentation/admin-guide/cgroup-v2.rst#L517-L564)).
  - `systemd-run --user` talks to "the service manager of the calling user" ([user-system-options.xml](https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/man/user-system-options.xml#L11-L12)). Here that manager runs only while Jacob is logged in.
  - With lingering, it is "spawned for the user at boot and kept around after logouts" ([loginctl.xml L186-L194](https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/man/loginctl.xml#L186-L194)).
- **With `loginctl enable-linger jacob`, run once,** each Trial runs as `systemd-run --user --scope`. The command then runs "by systemd-run itself as parent process" ([systemd-run.xml L70-L75](https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/man/systemd-run.xml#L70-L75)), so thirdshift's process handling stays as it is **[interpretation]**. The scope would set:
  - `MemoryHigh`, "the main mechanism to control memory usage";
  - `MemoryMax`, where the OOM killer is "invoked inside the unit";
  - `MemorySwapMax`, because the factory already uses 1.8 GiB of swap ([systemd.resource-control.xml L345-L455](https://github.com/systemd/systemd/blob/db11bab38ccf1ed257f310d29070843d4c58ea01/man/systemd.resource-control.xml#L345-L455)).

  The kernel adds `memory.oom.group`, which kills a cgroup "together or not at all" ([cgroup-v2.rst L1262-L1343](https://github.com/torvalds/linux/blob/e8f897f4afef0031fe618a8e94127a0934896aba/Documentation/admin-guide/cgroup-v2.rst#L1262-L1343)). A Trial that outgrows its cap then dies alone.
- **Protecting the factory directly** would mean `MemoryLow` on `cron.service`, which is a root drop-in.
- **Without any setup:**
  - a Trial can raise its own `oom_score_adj`, since "a process can always increase its own" setting ([proc_pid_oom_score_adj(5)](https://man7.org/linux/man-pages/man5/proc_pid_oom_score_adj.5.html)). That makes Trials the OOM killer's first choice;
  - thirdshift can also hold a Trial back while available memory is low **[interpretation]**.
- **rlimits don't substitute.** They are per process. `RLIMIT_AS` caps address space, not memory, and `RLIMIT_NPROC` counts every process of the UID, the factory's included ([getrlimit(2) n112-n131, n337-n344](https://git.kernel.org/pub/scm/docs/man-pages/man-pages.git/tree/man2/getrlimit.2?id=557f96fb1938cc82a543ae5af1dec920c282cf1e#n112)).
- **A Trial's real footprint is unmeasured** **[needs live check]**. "At most two" stays until it is measured.

### 7.3 Q5.6: the runner

- **A container sandbox makes both frameworks worth a little more, and changes neither verdict [interpretation].**
  - **Inspect** gains a per-Trial container lifecycle. But inspect_swe's agents are API-key only. A solver would `exec` thirdshift's flag inside the container with a Harness and a login copied in. The generated network has to be replaced so the Harness can reach its provider. The bwrap code it would save is the 100–200 lines eval-runners.md §1 counted.
  - **Harbor** becomes runnable. It brings a task format, verifiers and rewards, CPU and memory limits, an allowlist network mode, failure causes on `main`, ATIF and the Phoenix plugin. Its costs:
    - its built-in agents bypass thirdshift;
    - four of the six matching agents need API keys;
    - its Codex path copies `auth.json`;
    - a custom agent is undocumented;
    - its telemetry is on by default;
    - it pulls Python 3.12 with about 86 packages;
    - its metrics are a mean or pass@k, with no paired Arms and no Q4.4 interval.
- **(a), thirdshift's own runner in Rust, still fits best.** The parts worth borrowing are ideas, not code:
  - SWE-bench's time-safe clone and output markers;
  - Harbor's failure causes;
  - Harbor's three network modes, which match §3.2: public, none and allowlist;
  - Harbor's task layout: instruction, environment, tests and reward.

### 7.4 Q3.20: the implement suite

- **Start on the host's toolchains in bwrap (§5.3) [interpretation].**
  - Each Eval task is prepared and checked once at its base commit.
  - Each Trial gets a copy.
  - Code Graders run offline.
- **Move to per-repository images only when one of these happens:**
  - an Eval task's commit no longer builds on the host's toolchains;
  - a repository needs services, as keeplore might;
  - Replays move to a bigger machine.

  Images cost disk this host lacks, and a rebuild each time production's Harness versions change (§5.2).
- **The SWE-bench style still applies without Docker.** That means FAIL_TO_PASS and PASS_TO_PASS from the merged fix, `eval.sh`-style markers, and the time-safe clone.

### 7.5 Void Trials, the `infra_ok` idea

- **What the three projects do.**
  - **Phoenix's `infra_ok`** flags any exception, timeouts included, and keeps the reward.
  - **Harbor's metrics** count a trial without a verifier result as 0.
  - **SWE-bench** lists likely infrastructure failures but keeps them in the denominator.
  - Only **Harbor's new policy** both keeps a timeout as the agent's result and stops grading sandbox failures (§6.1).
- **A void Trial for thirdshift [interpretation].** A Trial is void when a rule says the measurement failed for a reason outside the Arm. The rules are fixed with the bar, before the Replay starts (Q3.4), so they can't be bent afterwards. Candidate causes:
  - the sandbox failed to start, or the Harness couldn't reach its provider;
  - the login failed or expired;
  - the provider answered with a server error before the agent did any work;
  - the plan-limit stop of Q3.7;
  - the host killed the Trial outside its own cgroup.
- **Not void:**
  - a timeout, as in Harbor's policy;
  - a context-window overflow;
  - a refusal;
  - anything the agent did inside its sandbox.
- **What happens to a void Trial.**
  - It is re-run as a fresh Trial, up to a cap recorded with the bar.
  - It is shown, with its cause, and counted per Arm.
  - It stays out of Q4.4's interval, rather than counting as a fail.
- **The cap case.** A Trial that hits its own memory cap is void, because Runs have no cap. If one Arm hits it repeatedly, that difference is reported as a finding.
- **Too many voids** make the Replay inconclusive, not passed.

### 7.6 Q5.5: Langfuse

- **Langfuse still needs too much memory** (§6.4).
- **A sandbox choice doesn't change that.** `stephens` already has Docker, and the `thirdshift` host wouldn't fit Langfuse in 8 GB even with Docker.
- **Q5.5's recommendation (a) is unaffected.**

## 8. Recommendation [interpretation]

**Keep bwrap, and make it whole.** For each Trial and each model Grader:

1. **Namespaces.**
   - bwrap with user, PID, IPC, UTS and network namespaces;
   - a fresh `/proc` and a tmpfs home;
   - `--die-with-parent`.
2. **Binds.**
   - read-write: the Trial's repository and the login file;
   - read-only: the system, the Harness's install, and the user-level files a Run's session reads.
3. **Network.**
   - **For the code-review and Spec-review suites:** an allowlisting proxy reached through a bound Unix socket and socat. Before relying on it, live-check each Harness behind `HTTPS_PROXY`, with the provider's hosts allowlisted.
   - **For any Harness that ignores proxies:** pasta joined to the namespace with `--no-map-gw -T none -U none -t none -u none`. That Harness's Trials keep the GitHub gap, and the gap is recorded.
   - **Code Graders:** no network at all.
4. **Memory.**
   - Run `loginctl enable-linger jacob` once.
   - Then put each Trial in a `systemd-run --user --scope` with `MemoryHigh`, `MemoryMax` and `MemorySwapMax`.
   - Until then, raise Trials' `oom_score_adj`.
5. **Logins.**
   - Claude through a `setup-token` token.
   - Codex and the others by binding the one real file.
   - No copies, and no whole homes.
6. **Void Trials** under rules fixed with the bar (§7.5).
7. **The implement suite** on the host's toolchains, with SWE-bench's time-safe clone (§7.4).
8. **No Docker on the `thirdshift` host for Replays now.** Revisit containers on any of the §7.4 triggers. Revisit Terminal-Bench as an outside yardstick for Model defaults only on another machine, with Harbor's telemetry off and outside Replays.

**Trade-offs.**
- **Costs.**
  - Roughly 400–650 lines of Rust for the sandbox, the network path and the scope, against eval-runners.md's 100–200 for bwrap alone **[interpretation]**.
  - One admin step (lingering), and perhaps passt with upstream's pasta profile.
  - A list of provider hosts per Harness to keep current.
- **Gains.**
  - A Trial differs from a Run only in what it can see and reach: same binaries, same userland, no daemon, no images.
  - Files, processes and network are enforced, not left to the prompt.
  - A Trial can't take the factory's memory.
- **What it gives up.**
  - Harbor's and Terminal-Bench's ecosystem, and Inspect's container lifecycle.
  - Reproducible images for old commits.
  - A sandbox someone else maintains.
- **Risks.**
  - A Harness that ignores `HTTPS_PROXY` falls back to pasta and keeps the GitHub gap.
  - Pasta may need upstream's profile on 24.04.
  - Claude's write method for its login is unknown. The `setup-token` route makes that moot.
  - Codex's shared `auth.json` under concurrency is already production's risk.

## 9. Uncertainties and live checks

- **[needs live check]** Each Harness inside bwrap with only its login bound:
  - does it log in, refresh, and write the refresh back?
  - how does Claude write `.credentials.json`?
  - where do Grok, Muse and agy keep their login files?
- **[needs live check]** Each Harness through `HTTPS_PROXY` over a Unix-socket relay, and the hosts each provider needs: OpenCode, Muse and agy are undocumented. This includes Claude's `claude.ai` and `platform.claude.com` for OAuth ([sandbox-environments](https://code.claude.com/docs/en/sandbox-environments)).
- **[needs live check]** pasta's attach mode with Ubuntu 24.04's passt package, which has no pasta profile, and DNS through `--dns-forward` to the 127.0.0.53 stub.
- **[needs live check]** `systemd-run --user` from a cron job after lingering, and a Trial's real memory use beside two building Runs.
- **[needs live check]** Interruption through bwrap. thirdshift sends Codex SIGINT, then the group SIGTERM and SIGKILL. Does each signal reach the Harness, and does the PID namespace's end kill what it left behind?
- **[needs live check]** Whether Codex's own sandbox now starts on this host through `/usr/bin/bwrap`. That would bear on ADR 0012.
- **Moving targets.** Harbor's error policy exists only on `main`, and Phoenix's `infra_ok` may change with it.
- **No primary overhead figures** were found for bwrap, pasta, slirp4netns, Podman or nspawn. gVisor's are old benchmark output.
- **The line estimates** in §8 come from reading code, not from a prototype.
- **Unchecked loopback listeners.** The loopback listeners besides the Console, Caddy and the resolver weren't traced to their owners.

## 10. Live checks, 2026-10-21

#1009's checks, as far as they could go without real logins. They ran on the `thirdshift` host with test processes, fake credentials and fresh homes. No real login was bound or read, and nothing was installed. Where a §9 item is answered here, this section says so.

- **Isolation (§2).** Inside bwrap with `--unshare-pid --unshare-net --new-session`, a process:
  - saw 5 PIDs, against the host's 198;
  - had no network interface but `lo`;
  - couldn't reach the Console on 127.0.0.1:8711;
  - couldn't see `/home/jacob`.

  Confirmed by a sandboxed shell listing `/proc`, `/proc/net/dev` and `/home`, and by `curl` to the Console.
- **Interruption through the PID namespace (§9).** thirdshift's stop path snapshots `ps -A -o pid=,ppid=,pgid=`, and signals every process group among the Harness's descendants.
  - **Signals arrive.** Done that way, SIGTERM and SIGINT both reached a trap inside the sandbox. That held with and without `--new-session`. `--new-session` puts the inner process in a group of its own, which the tree walk still finds.
  - **Nothing outlives it.** SIGKILL to the outer bwrap alone took down every process inside it.

  Confirmed with a stand-in Harness that records the signal it gets.
- **The relay and the proxy (§3.2).** The setup: a CONNECT proxy outside the sandbox, on a Unix socket bound into it, with `socat` relaying 127.0.0.1:3128 inside to that socket. Through it:
  - allowed hosts were reached: `api.anthropic.com` answered 405, and `api.openai.com` 401;
  - GitHub was refused, as not allowed;
  - an allowlisted `localhost` was refused, because it resolves to `::1`;
  - a direct connection, without the proxy, had no route at all.

  A Unix socket's path is limited to 108 bytes, so the launcher can't put its socket deep under an Experiment's folder. A short directory, such as one under `/tmp`, works. Confirmed by `curl` and the proxy's log.
- **Each Harness through `HTTPS_PROXY` (§3.2, §9).** Each ran in that sandbox, with a fresh home and a fake credential, and all six used the proxy:

  | Harness | Hosts it contacted | Notes |
  |---|---|---|
  | Claude Code 2.1.307 | `api.anthropic.com` | 401 on the fake token. |
  | Codex 0.163.0 | `api.openai.com`, `chatgpt.com`, `github.com`, `api.github.com`, `files.openai.com` | The GitHub hosts are its plugin sync at startup. With them and `files.openai.com` refused, it carried on to the API. |
  | Grok Build 1.0.62 | `api.x.ai` | |
  | Muse Code | `api.meta.ai` | Only with a key set. Without one, it checks for credentials locally and never reaches the network. |
  | agy 1.3.3 | `antigravity-unleash.goog`, `play.googleapis.com` | With no login, it opens a Google sign-in and waits 60 seconds, so its API host wasn't reached. A Trial whose agy login is missing waits that long before failing. |
  | OpenCode 2.0.24 | `opencode.ai`, `models.opencode.ai` | It calls its own local server over HTTP, so it needs `NO_PROXY=127.0.0.1,localhost`. Without that, it looped until it timed out. With no provider configured, it answered on a free model, so a Trial must name its provider. |

  Confirmed by the proxy's log. The hosts used with a real subscription login aren't in it: for Codex's ChatGPT login, `chatgpt.com`, and its refresh, `auth.openai.com`, from Codex's source. agy's API host is also missing.
- **Web search off (§3.2).** Production sessions keep web search on, since thirdshift passes these switches only to release summaries. For Trials:
  - **Claude:** `--disallowedTools WebSearch WebFetch`.
  - **Codex:** `-c web_search="disabled"`.
  - **Grok:** `--disable-web-search`.
  - **Muse:** `--disable-web-tools`.
  - **OpenCode:** `OPENCODE_CONFIG_CONTENT='{"permission":{"webfetch":"deny","websearch":"deny"}}'`, which `opencode debug config` resolves to deny for both.
  - **agy: still open.** No switch was found in its help or its settings. Its binary carries a `google_search` grounding tool, which runs at Google and so gets past any proxy.

  Confirmed by each CLI's help, thirdshift's adapters, and OpenCode's resolved config.
- **Memory with a fresh home (§4).** Every Harness kept its state under the fresh home, and none needed the real home to start:
  - Claude: `~/.claude` and `~/.claude.json`;
  - Codex: its `CODEX_HOME`, including `memories_1.sqlite`;
  - Grok: `~/.grok`;
  - Muse: `~/.local/share/muse`;
  - agy: `~/.gemini/antigravity-cli`, including `brain` and `implicit`;
  - OpenCode: `~/.local/share/opencode`, `~/.config/opencode` and `~/.cache/opencode`.

  So a fresh home per Trial starts each Harness with no memory. Claude's `--bare` would also turn auto-memory off, but it skips hooks and CLAUDE.md and accepts only an API key, so it doesn't suit Trials. Confirmed by listing what each left in its home.
- **A session's memory (§7.2).** A snapshot of the Harness sessions running at the time, from `ps` resident sizes (names only, no command lines):

  | Harness | Memory in use |
  |---|---|
  | Claude | about 480 MiB, most likely the session that ran these checks, with a very large context |
  | Codex | about 200 MiB |
  | Grok | about 150 MiB |
  | OpenCode | about 80 MiB |

  6.6 GiB was available, and no build was running. A Trial's memory beside two building Runs is still open.
- **Harnesses update themselves (Q7.2).** During the day, Claude Code went from 2.1.306 to 2.1.307, and Grok Build from 1.0.50 to 1.0.62. That backs fixing each Harness's binary when an Experiment starts (#1002).
- **Still open, for a session with the real logins:**
  - whether each Harness logs in, refreshes and saves its login inside bwrap, with only its login file bound, and Claude with a `claude setup-token` token (#1011);
  - the hosts each provider needs with a real subscription login;
  - how to turn agy's web search off;
  - a Trial's memory beside two building Runs.

## 11. Live checks with the real logins, 2026-10-23

The rest of #1009, on the `thirdshift` host. Each Harness ran in the §10 sandbox:
- `bwrap --unshare-all --new-session`, with a fresh home at `/home/jacob`;
- the real login file bound read-write at its own path, and nothing else of the real home but the Harness's binary, and for OpenCode, its config;
- the network only through the relay, to a proxy that allowed every host and logged each one.

The prompt was a one-word reply. No login's contents were printed or recorded, only each file's inode, size and modification time, before and after. Claude's token went from the Credentials into the sandbox's environment, never onto a command line.

- **Logins inside bwrap (§4, §9).**

  | Harness | Login bound | Result | Written back |
  |---|---|---|---|
  | Claude Code 2.1.309 | none; `CLAUDE_CODE_OAUTH_TOKEN` from the Credentials' `claude.setup_token` | 401, "OAuth access token is invalid". The same outside the sandbox, so the saved token is bad, not the sandbox: it is 76 characters, most likely half of a wrapped paste. | — |
  | Codex 0.163.0 | `~/.codex/auth.json`, a ChatGPT login | Answered. | No. Its access token expires on 2026-10-24 at 20:27, so no refresh was due. |
  | Grok Build 1.0.62, then 1.0.67 | `~/.grok/auth.json` and `auth.json.lock` | 1.0.62 signed in, then got 402, "Grok Build usage balance exhausted". After the balance was topped up, 1.0.67 answered. | Not inside the sandbox, though 1.0.62 rewrote the lock in place. Outside it, at 15:09, the host's `auth.json` was replaced: it has a new inode. |
  | Muse Code 1.4.12 | `~/.config/muse/auth.json` and `.auth.json.lock` | Answered. | No. |
  | agy 1.3.3 | `~/.gemini/antigravity-cli/antigravity-oauth-token`, with no D-Bus | Answered. | Yes: it refreshed through `oauth2.googleapis.com` and wrote the file in place, keeping its inode, while its size went from 1688 to 571 bytes. agy outside the sandbox still signed in with it afterwards. |
  | OpenCode 2.0.24 | `opencode.json`, then also a copy of `~/.local/share/opencode/opencode.db` | With the config alone: "No api key passed in". Its PrimaLabs key isn't in a file. `opencode auth list` says it's stored, and the database has a `credential` table beside its sessions. With a copy of the database, it answered on PrimaLabs' MiMo-V2.6-Pro. The copy was deleted afterwards. | — (an API key doesn't refresh) |

  What follows:
  - **Single-file binds work for Codex, Muse and agy.** Every write seen inside the sandbox was in place. A test file confirmed both halves of §4's rule: an in-place write through a single-file bind reaches the host's file, and a rename onto it fails with "Device or resource busy".
  - **Grok saves its login by rename.** Its `auth.json` got a new inode when its login was renewed outside the sandbox. So a Grok that renews its login inside the sandbox can't save it through a single-file bind. Binding `~/.grok` instead would show the Trial Grok's sessions and memory. #1012 needs a third way [interpretation]:
    - renew Grok's login before each Trial;
    - or give Grok a `GROK_HOME` of its own, holding only the login and its lock.
  - **Bind the lock files too.** Grok and Muse keep one beside the login. A lock left in the fresh home would lock nothing against the factory's own sessions [interpretation].
  - **Codex needs its release folder.** It looks for `codex-code-mode-host` beside its binary, and says code mode fails closed without it. So the sandbox binds the binary's folder, not the binary alone.
  - **OpenCode can't be given just its login.** Its credentials share a database with its sessions, so the copy used here also carried its sessions. A Trial mustn't get that. `OPENCODE_AUTH_CONTENT`, which §4 relied on, is no longer in its binary, though `OPENCODE_DB` is. Two ways remain [interpretation]:
    - give the Trial's config the key through OpenCode's `{env:…}` substitution;
    - or give it a database of its own through `OPENCODE_DB`.
  - **Codex's refresh wasn't seen.** Its source writes in place (§4), and the bind carries in-place writes, but a sandboxed refresh still needs a run after 20:22 on 2026-10-24.

  Confirmed by each Harness's output and exit status, `stat` before and after, and the same Claude token failing outside bwrap.
- **Hosts with the real logins (§3.2, §9).** From the proxy's log. These are the inputs to #1013's allowlist.

  | Harness | Hosts |
  |---|---|
  | Claude Code | `api.anthropic.com`. The token was refused, so any hosts after sign-in are missing. |
  | Codex | `chatgpt.com`, `ab.chatgpt.com`, and `sdmntprcentralus`, `sdmntprnorthcentralus` and `sdmntprsouthcentralus.oaiusercontent.com`. With no refresh, `auth.openai.com` wasn't contacted. Unlike §10, no GitHub host was contacted. |
  | Grok Build | `api.x.ai`, `cli-chat-proxy.grok.com`, `grok.com`, and `api.mixpanel.com` (analytics). 1.0.62 also contacted `auth.x.ai`. |
  | Muse Code | `api.meta.ai`, `lookaside.facebook.com` |
  | agy | `daily-cloudcode-pa.googleapis.com` and `aicode.googleapis.com` (the API), `oauth2.googleapis.com` (the refresh), `www.googleapis.com`, `play.googleapis.com`, `lh3.googleusercontent.com`, `antigravity-unleash.goog`, and `antigravity-cli-auto-updater-974169037036.us-central1.run.app` |
  | OpenCode | `api.primalabs.ai`, `models.opencode.ai`, with the provider configured here |

  An allowlist can refuse the analytics and updater hosts. Whether each Harness carries on without them wasn't tested.
- **agy's web search can't be turned off (§10).** agy ran its `search_web` tool and answered with Rust's version. It did so in print mode without `--dangerously-skip-permissions`, in each of these setups:
  - its default `request-review` preset;
  - `permissions.deny` holding `read_url(*)` and `execute_url(*)` ([permissions](https://antigravity.google/docs/permissions/));
  - the `strict` preset.

  agy's log showed it had read each `settings.json`. Naming the tool fails: it logs "ignoring invalid deny entry" for `search_web`, and "unknown action" for `search_web(*)`. The search goes through its API host, `daily-cloudcode-pa.googleapis.com`, so no allowlist can separate it from the model's own calls. So an agy Trial always has web search [interpretation]. A suite that must keep later fixes out has two choices: leave agy out, or void any agy Trial whose stream shows a `search_web` call. Confirmed by agy's `stream-json` output and its log.
- **A Trial's memory (§7.2, §9).** From each systemd scope's `anon` in `memory.stat`, sampled every half second:
  - **A Harness session**, through a one-word reply, including the proxy:
    - Codex 101 MiB;
    - Grok 55 MiB;
    - Muse 56 MiB;
    - agy 199 to 216 MiB;
    - OpenCode 252 MiB;
    - Claude 133 MiB, before it failed.

    §10's 480 MiB for a long Claude session is the higher figure to plan on.
  - **thirdshift's build:** a fresh `cargo test --no-run --locked` took 228 seconds and about 11 minutes of CPU. Its peak was 1535 MiB of anonymous memory. Counting page cache, it was 3722 MiB, which is mostly reclaimable.
  - **The test run that follows,** `cargo test --locked`, passed. It peaked at 257 MiB, or 406 MiB with page cache. It took 3457 seconds, because it ran beside the Run below at a load average near 20.
  - **Beside a building Run.** While Ticket run #1082 was in its implement stage, and the test run above was going, the host kept these over a minute of samples:
    - at least 5447 MiB available;
    - swap use unchanged;
    - a load average of about 21 on 4 cores.

    No moment with two building Runs came up.

  So a Trial of the implement suite needs about 2 GiB at its build's peak [interpretation]. Two building Runs at the same peak add about 3 GiB, which fits in 7.7 GiB with 4 GiB of swap. CPU, not memory, is what a Trial beside two Runs runs short of. That bears on #1015 and #1152.
- **`systemd-run --user` works (§9).** With lingering on, a `--user` scope and a `--user` service both started, and the user manager has the memory controller. Two catches for #1015:
  - `systemd-run --wait --collect` reported "Memory peak: 0B";
  - `memory.peak` counts page cache.

  So the launcher should sample `anon` in `memory.stat` itself. From a cron job wasn't tried.
- **Harnesses update themselves (Q7.2), again.** Since §10, Claude Code has gone from 2.1.307 to 2.1.309. Grok Build went from 1.0.62 to 1.0.67 between two runs of this check.
- **Still open:**
  - Claude with a working setup token: save it again with `thirdshift setup`, then rerun its row;
  - OpenCode with only its key, by either of the two ways above;
  - a sandboxed Codex refresh, after 20:22 on 2026-10-24.

## Sources

thirdshift:
- the [#698 grilling](../../.scratch/grill-with-docs/done/698-evals/README.md);
- [eval-runners.md](eval-runners.md), [eval-viewers.md](eval-viewers.md), [eval-tools-landscape.md](eval-tools-landscape.md) and [harness-candidates.md](harness-candidates.md);
- [ADR 0012](../adr/0012-factory-skills-linked-into-the-worktree-codex-unsandboxed.md) and [ADR 0013](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md);
- [the Security skill](../../skills/thirdshift-security-audit/SKILL.md);
- `src/session.rs` and `src/process.rs` at `dfadafa`.

Linux sandboxing:
- [bubblewrap `8e51677`](https://github.com/containers/bubblewrap/tree/8e51677abd7e3338e4952370bf7d902e37d8cbb6) and [`362936a`](https://github.com/containers/bubblewrap/tree/362936aea0d5c6e70f7af75d2d44b5a914cb4ba2);
- passt at [`1e6f92b`](https://passt.top/passt/tree/?id=1e6f92b995a9f7bff83b8b7a4b51782804fc23c5) and [`cba3570`](https://passt.top/passt/tree/?id=cba357068dc586e7540971eb40bb56c5e62c4de1);
- [slirp4netns `09e31e9`](https://github.com/rootless-containers/slirp4netns/tree/09e31e92fa3d2a1d3ca261adaeb012c8d75a8194);
- Podman [`8d2b55d`](https://github.com/containers/podman/tree/8d2b55ddde1bc81f43d018dfc1ac027c06b26a7f) and [`40030f4`](https://github.com/containers/podman/tree/40030f4073bd4e1bf4f2fd9df2f6952b4d9d350a), and [containers/common `215e898`](https://github.com/containers/common/tree/215e898615b7de5f789be60784b8303f2ab6de3d);
- [Docker docs `226023f`](https://github.com/docker/docs/tree/226023fc89bdeb3f051c4c8c697d04ed5f5bcd2c), [moby `0367cbe`](https://github.com/moby/moby/tree/0367cbeb49b9d98cb0b4f51e95f7b54b4803e76c) and [rootlesskit `888c318`](https://github.com/rootless-containers/rootlesskit/tree/888c3189e9cdf34dcdb19011e1c7c34eb94b5393);
- systemd [`db11bab`](https://github.com/systemd/systemd/tree/db11bab38ccf1ed257f310d29070843d4c58ea01) and [`73612ea`](https://github.com/systemd/systemd/tree/73612ea502d53baf9b0df4336cfc296a7ee0d524);
- [gVisor `60faa65`](https://github.com/google/gvisor/tree/60faa657ca99f3398e04c8dbc580a848df20f2f2);
- [Firecracker `95f868c`](https://github.com/firecracker-microvm/firecracker/tree/95f868c8e345b1cc8faccd1a3c910b4989dc3f58), [Cloud Hypervisor `038ce5a`](https://github.com/cloud-hypervisor/cloud-hypervisor/tree/038ce5a72d761380a02f714a4eb4ca63e0ea073d) and [Kata `bd75006`](https://github.com/kata-containers/kata-containers/tree/bd75006ebc9dea6fcb04d8e5ae4e40a3f9316fbf);
- [Linux v6.8 `e8f897f`](https://github.com/torvalds/linux/tree/e8f897f4afef0031fe618a8e94127a0934896aba) and [Ubuntu-6.8.0-124.124 `97d6d07`](https://git.launchpad.net/~ubuntu-kernel/ubuntu/+source/linux/+git/noble/tree/?id=97d6d07ae111bd2656537f792bae578399e75c7a);
- man-pages [6.7](https://git.kernel.org/pub/scm/docs/man-pages/man-pages.git/tree/?id=557f96fb1938cc82a543ae5af1dec920c282cf1e) and [6.19 on man7.org](https://man7.org/linux/man-pages/);
- [Ubuntu 24.04 release notes](https://discourse.ubuntu.com/t/noble-numbat-release-notes/39890) and [LP #2077158](https://bugs.launchpad.net/ubuntu/+source/passt/+bug/2077158);
- [Caddy's website `1957519`](https://github.com/caddyserver/website/tree/19575198cce1ea5f1bb37895ef3f9f2ec0f1997a).

Harnesses:
- Claude Code docs, fetched 2026-10-21: [sandboxing](https://code.claude.com/docs/en/sandboxing), [sandbox environments](https://code.claude.com/docs/en/sandbox-environments), [authentication](https://code.claude.com/docs/en/authentication), [env vars](https://code.claude.com/docs/en/env-vars), [memory](https://code.claude.com/docs/en/memory), [network config](https://code.claude.com/docs/en/network-config) and [Agent SDK secure deployment](https://code.claude.com/docs/en/agent-sdk/secure-deployment);
- [sandbox-runtime `aceba12`](https://github.com/anthropics/sandbox-runtime/tree/aceba124fd725f4a43d7016ed3f5651d61f83115);
- [Codex `8308618`](https://github.com/openai/codex/tree/83086188ae0614bd882818ff1aa23f3194962012) and its docs: [CI/CD auth](https://learn.chatgpt.com/docs/auth/ci-cd-auth), [approvals and security](https://learn.chatgpt.com/docs/agent-approvals-security) and [permissions](https://learn.chatgpt.com/docs/permissions);
- [OpenCode `e3c4eb2`](https://github.com/anomalyco/opencode/tree/e3c4eb242585dcd77be1a867391b7c25fd47ce52);
- Grok Build docs: [sandbox](https://docs.x.ai/build/features/sandbox), [settings](https://docs.x.ai/build/settings/reference) and [enterprise](https://docs.x.ai/build/enterprise);
- Muse Code docs: [permissions](https://dev.meta.ai/docs/muse-code/permissions) and [auth](https://dev.meta.ai/docs/muse-code/auth);
- Antigravity docs: [install](https://antigravity.google/docs/install), [CLI features](https://antigravity.google/docs/cli/features) and [settings](https://antigravity.google/docs/settings).

Eval tools:
- [Harbor `6c9d3d6`](https://github.com/harbor-framework/harbor/tree/6c9d3d6609854375005a7b5c8f1906371d64c6ff) and [v0.24.0 `b53b813`](https://github.com/harbor-framework/harbor/tree/b53b8134e1241686dca7759af188f987ecc48e8b);
- [Phoenix `f6a0231`](https://github.com/Arize-ai/phoenix/tree/f6a023154da2c899e6937d49b37b2ebce8e8609b);
- [Terminal-Bench 2 `2fd12b8`](https://github.com/harbor-framework/terminal-bench-2/tree/2fd12b88aafdd04a52c298e3940bcb189f9766d6);
- [LiteLLM `052b5a2`](https://github.com/BerriAI/litellm/tree/052b5a2169d8d3082e1d66e69f200a72b0c1e274);
- [SWE-bench `490635b`](https://github.com/SWE-bench/SWE-bench/tree/490635b2e9e775dca1e1d6b40ce9dbcff91e780f), [swe-bench-tasks `3d07b46`](https://github.com/SWE-bench/swe-bench-tasks/tree/3d07b464b7b311a0cbfb5ed5b2d8a3b96f84a33d) and [swe-bench-multilingual-tasks `6e08cbc`](https://github.com/SWE-bench/swe-bench-multilingual-tasks/tree/6e08cbcd763c869abd7aae4bcb9e1b3991180777);
- [Inspect AI `386e099`](https://github.com/UKGovernmentBEIS/inspect_ai/tree/386e09942510e93de2db6d108f24f62f65eecbdc) and [inspect_swe `377b111`](https://github.com/meridianlabs-ai/inspect_swe/tree/377b1112ef7177190844100825baba64706536bc);
- [langfuse-docs `5ee3f4e`](https://github.com/langfuse/langfuse-docs/tree/5ee3f4e54283dedf26780ac6f38072012b2c2d88) and [ClickHouse `1b29cde`](https://github.com/ClickHouse/ClickHouse/tree/1b29cdec0333ca779336d4a688b176644b80bcc1);
- Docker Hub's API for image sizes, read on 2026-10-21.
