# Browser WebGL2 and WebGPU Rendering in Linux VMs and Sandboxes

Research date: 2026-11-13

**Question.** Following #1665 ([`docs/adr/0003-static-binaries-through-github-releases.md`](../adr/0003-static-binaries-through-github-releases.md)) and [`docs/research/linux-on-macbook.md`](linux-on-macbook.md), what are the viable options, performance trade-offs, and isolation implications for running headless browser 3D workloads (Three.js WebGL2 and WebGPU under Playwright) across Apple Silicon Linux VMs and Linux `bwrap` sandboxes?

**Method.**

- Provisioned an isolated Fedora 44 `aarch64` Lima VM (`--vm-type=krunkit --mount-none`, `krunkit` 1.3.2, `libkrun` 1.19.8, `molten-vk` 1.4.2, 6 vCPUs, 12 GiB RAM) on macOS Tahoe 26.5.2 (Apple M5 Pro, Lima 2.2.1).
- Tested Vulkan compute (`vulkaninfo` 1.4.341) and headless Chromium (`156.0.8078.79`) across ANGLE/Vulkan, ANGLE/EGL with Mesa Zink, ANGLE/EGL with Virgl, Dawn WebGPU, `SwiftShader`, and `Mesa llvmpipe`, plus a dedicated macOS headless Chrome instance driven from inside the Lima guest over the Chrome DevTools Protocol (CDP).
- Measured synchronized Three.js WebGL2 frame times (`renderer.render` followed by a `gl.readPixels` fence) and PNG `page.screenshot()` times across two workloads: a 720p tactical maquette scene (~38k triangles, `1024²` `PCFSoftShadowMap`) and a 1080p heavy PBR scene (~581k triangles, `2048²` `PCFSoftShadowMap`).
- Audited `thirdshift`'s execution and sandbox boundaries ([`docs/adr/0013-every-harness-runs-unattended-and-fully-trusted.md`](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md), [`src/sandbox.rs`](../../src/sandbox.rs), [`src/sandbox/code_grader.rs`](../../src/sandbox/code_grader.rs), [`src/sandbox/triage.rs`](../../src/sandbox/triage.rs), [`src/setup/sandbox.rs`](../../src/setup/sandbox.rs), [`src/proxy.rs`](../../src/proxy.rs)) and empirically tested `bwrap` GPU device/sysfs bind permutations alongside `systemd-run --user --scope`.
- Traced each failure mode to upstream [Lima](https://lima-vm.io/docs/config/vmtype/krunkit/), [`krunkit`](https://github.com/containers/krunkit), [`libkrun`](https://github.com/containers/libkrun), [MoltenVK](https://github.com/KhronosGroup/MoltenVK), [Apple `Hypervisor.framework`](https://developer.apple.com/documentation/hypervisor), [Mesa Venus](https://docs.mesa3d.org/drivers/venus.html), [Mesa Zink](https://docs.mesa3d.org/drivers/zink.html), [`libdrm`](https://gitlab.freedesktop.org/mesa/drm/-/blob/main/xf86drm.c), and [Playwright](https://playwright.dev/docs/api/class-browsertype#browser-type-connect-over-cdp) sources and documentation.

**Out of scope.** Changing `thirdshift`'s `bwrap` bind flags or adding host-CDP browser support (open follow-up decisions tracked in [#2173](https://github.com/JacobStephens2/thirdshift/issues/2173)).

## TL;DR

- **Software WebGL2 (`SwiftShader`) inside a standard Lima `vz` VM or Linux server is the right default for ordinary 3D Playwright checks and static PR screenshots.** On 6 Apple M5 Pro vCPUs, `SwiftShader` renders a 720p Three.js shadow-mapped maquette (~38k triangles) in **84.62 ms/frame** (11.8 fps) with **±1/255 RGBA pixel parity** against native Apple Metal (`gl.getError() === 0`). It preserves `--mount-none` VM isolation and works out of the box with standard `chromium.launch(...)` suites.
- **Lima `krunkit` (`Virtio-GPU Venus` on macOS) works for Vulkan compute, not for Chromium WebGL2 or WebGPU.** After replacing stock Fedora 44 Mesa (`26.2.6`, whose 4 KiB-aligned blob mappings fail Apple Silicon's 16 KiB `hv_vm_map` requirement) with the `slp/mesa-libkrun-vulkan` COPR (`25.3.6-102.fc44`), `/dev/dri/renderD128` exposes `Virtio-GPU Venus (Apple M5 Pro)`. Even then, every Chromium GPU rendering path fails initialization or falls back to CPU rasterization because macOS `MoltenVK` lacks Linux `SYNC_FD` semaphore import (`VK_KHR_external_semaphore_fd`, gating `VK_KHR_swapchain` in Mesa Venus) and `VK_EXT_robustness2` `nullDescriptor` (required by Mesa Zink), while `libkrun` on macOS provides no host Virgl OpenGL context.
- **Driving headless Chrome on macOS over CDP (`connectOverCDP`) delivers native `ANGLE Metal` speed (1.31 ms/frame at 720p, 3.67 ms/frame at 1080p — 65×–153× faster than `SwiftShader`), at the cost of crossing the VM isolation boundary and requiring custom test wiring.** `bwrap`-sandboxed **Trials** and **Graders** cannot reach it (`--unshare-net` and `src/proxy.rs` host/loopback refusal), whereas unsandboxed **Ticket runs** and **Spec runs** inside the VM can reach the host browser on `192.168.5.2:<port>`.
- **On native Linux GPU hosts (such as cloud NVIDIA or AMD machines), exposing the GPU inside `bwrap` requires sysfs binds alongside `/dev/dri`.** Binding `--dev-bind-try /dev/dri /dev/dri` alone still fails (`ERROR_INITIALIZATION_FAILED`) because `libdrm`'s `drmGetDevices2()` walks `/sys/dev/char` and `/sys/devices`; adding `--ro-bind /sys/dev/char /sys/dev/char --ro-bind /sys/devices /sys/devices` (plus `/dev/nvidia*` device nodes on NVIDIA hosts) makes Vulkan device enumeration succeed cleanly, including under `--unshare-user --cap-drop ALL`.

---

## 1. Lima `krunkit` (`Virtio-GPU Venus`) on Apple Silicon

Lima 2.0+ ships an experimental `krunkit` VM driver ([Lima: `krunkit`](https://lima-vm.io/docs/config/vmtype/krunkit/)) built on [`krunkit`](https://github.com/containers/krunkit) and [`libkrun`](https://github.com/containers/libkrun) over Apple's [`Hypervisor.framework`](https://developer.apple.com/documentation/hypervisor). Inside the Linux guest, [Mesa's Venus driver](https://docs.mesa3d.org/drivers/venus.html) serializes Vulkan commands across `virtio-gpu` to `libkrun` on macOS, which replays them through [MoltenVK](https://github.com/KhronosGroup/MoltenVK) onto Metal.

### 1.1 Guest 4 KiB vs. host 16 KiB page-alignment requirement

Booting `template:fedora` (Fedora 44 `aarch64`, kernel `6.19.10-300.fc44.aarch64` with 4 KiB pages) under `--vm-type=krunkit` exposes `/dev/dri/renderD128` (`crw-rw-rw-`), yet stock Fedora `mesa-vulkan-drivers` (`26.2.6-1.fc44`) fails `vkCreateInstance` with `ERROR_OUT_OF_HOST_MEMORY`:

- Guest `dmesg` logs `[drm:virtio_gpu_dequeue_ctrl_func [virtio_gpu]] *ERROR* response 0x1200 (command 0x208)` (`VIRTIO_GPU_CMD_RESOURCE_MAP_BLOB` → `VIRTIO_GPU_RESP_ERR_OUT_OF_MEMORY`).
- Host `krunkit.log` logs `ERROR krun_vmm ... Error adding memory map: MemoryMap`.
- **Root cause:** Apple Silicon macOS uses 16 KiB virtual memory pages, and `Hypervisor.framework` requires host-mapped guest memory regions (`hv_vm_map`) to be 16 KiB-aligned ([Apple `Hypervisor.framework`](https://developer.apple.com/documentation/hypervisor)). Stock upstream Mesa on a 4 KiB-page Linux kernel sub-allocates 4 KiB-aligned ring/blob offsets that `hv_vm_map` rejects.
- **Fix for Vulkan compute:** Installing the patched Mesa packages recommended by [Lima's `krunkit` documentation](https://lima-vm.io/docs/config/vmtype/krunkit/) from [`slp/mesa-libkrun-vulkan`](https://copr.fedorainfracloud.org/coprs/slp/mesa-libkrun-vulkan/) (`25.3.6-102.fc44`) aligns blob mappings to 16 KiB, after which `vulkaninfo --summary` reports:
  ```text
  GPU0:
      deviceName         = Virtio-GPU Venus (Apple M5 Pro)
      apiVersion         = 1.2.0
      driverID           = DRIVER_ID_MESA_VENUS
      driverInfo         = Mesa 25.3.6
  ```

### 1.2 Why Chromium WebGL2 and WebGPU still fail on macOS `krunkit`

Even with `Virtio-GPU Venus (Apple M5 Pro)` working for headless Vulkan compute (`vulkaninfo`, `llama.cpp`), testing every GPU backend in Chromium `156.0.8078.79` shows that browser rendering cannot use it on macOS hosts today:

| Chromium GPU path | Launch flags / environment | Observed result | Upstream root cause |
| --- | --- | --- | --- |
| **ANGLE → Vulkan** | `--headless=new --ozone-platform=headless --use-gl=angle --use-angle=vulkan` (with and without `MESA_VK_WSI_HEADLESS_SWAPCHAIN=1`) | Fails: `vk_renderer.cpp:282: Extension not supported: VK_KHR_swapchain`; WebGL context creation fails unless SwiftShader fallback is enabled | Upstream Mesa Venus ([`src/virtio/vulkan/vn_physical_device.c`](https://gitlab.freedesktop.org/mesa/mesa/-/blob/main/src/virtio/vulkan/vn_physical_device.c)) gates `KHR_swapchain` advertisement on `renderer_sync_fd.semaphore_importable`. Host `MoltenVK` on macOS has no Linux `SYNC_FD` external semaphore primitive (`VK_KHR_external_semaphore_fd`), so Venus never exposes `VK_KHR_swapchain`. |
| **ANGLE/EGL → Mesa Zink** | `--use-gl=angle --use-angle=gl-egl` with `GALLIUM_DRIVER=zink` | Fails: `MESA: error: Zink requires the nullDescriptor feature of KHR/EXT robustness2` | [Mesa Zink](https://docs.mesa3d.org/drivers/zink.html) requires `VK_EXT_robustness2`'s `nullDescriptor` feature for OpenGL semantics, which `MoltenVK` over Metal does not advertise through Venus. |
| **ANGLE/EGL → Virgl** | `--use-gl=angle --use-angle=gl-egl` | `libEGL warning: egl: failed to create dri2 screen`; falls back to `Mesa llvmpipe` | `libkrun` on macOS initializes virtio-gpu with Venus/Vulkan rather than a host Virgl OpenGL renderer ([`containers/libkrun`](https://github.com/containers/libkrun)). |
| **WebGPU (Dawn)** | `--enable-unsafe-webgpu --enable-features=Vulkan,WebGPU` | `navigator.gpu.requestAdapter()` falls back to `google / swiftshader` | Dawn's Linux Vulkan backend checks for the same external-memory / swapchain extensions missing on Venus over MoltenVK. |

*(Playwright flag note: `chromium.launch({ headless: true })` appends `--use-angle=swiftshader-webgl` after caller `args`, silently overriding `--use-angle=vulkan`. Testing explicit ANGLE backends in Playwright requires `headless: false`, `ignoreDefaultArgs: ['--enable-unsafe-swiftshader']`, and passing `'--headless=new'` inside `args`.)*

### 1.3 Operational trade-offs vs. Lima `vz`

Beyond offering no browser WebGL2 or WebGPU acceleration over `vz`, Lima's `krunkit` driver remains marked experimental ([Lima: `krunkit`](https://lima-vm.io/docs/config/vmtype/krunkit/)). Unlike `vz` (`Virtualization.framework`), it relies on an external `lima-driver-krunkit` helper process for usernet and vsock multiplexing (which logged transient `vsock::muxer` stream errors under load during testing). For local Apple Silicon Linux VMs, Lima `vz` ([`docs/research/linux-on-macbook.md` §2.1](linux-on-macbook.md#21-lima-limactl-with-apple-vz-and-ubuntu-2404-lts--recommended)) remains strictly preferable unless testing Vulkan compute specifically.

---

## 2. macOS Host Headless Chrome over CDP

Running headless Chrome natively on macOS (`--headless=new --remote-debugging-address=127.0.0.1 --remote-debugging-port=<port> --user-data-dir=<tempdir>`) and connecting from Playwright inside the Lima guest via [`chromium.connectOverCDP`](https://playwright.dev/docs/api/class-browsertype#browser-type-connect-over-cdp) bypasses Linux guest virtualization entirely and renders on `ANGLE (Apple, ANGLE Metal Renderer: Apple M5 Pro)`.

### 2.1 Networking mechanics between Lima and macOS

- **Guest → host CDP (`192.168.5.2:<port>`):** Lima maps the macOS host's loopback to `host.lima.internal` (`192.168.5.2` on usernet / `vzNAT`) ([Lima: Network](https://lima-vm.io/docs/config/network/)). Calling `connectOverCDP('http://host.lima.internal:<port>')` by hostname fails with HTTP `500` because Chrome's DevTools HTTP server rejects non-`localhost`, non-IP `Host` headers to prevent DNS-rebinding attacks; resolving `host.lima.internal` to `192.168.5.2` first and calling `connectOverCDP('http://192.168.5.2:<port>')` succeeds without weakening `--remote-debugging-address=127.0.0.1` on the Mac.
- **Host Chrome → guest dev server (`127.0.0.1:<app-port>`):** When a test server inside the Lima VM binds `0.0.0.0:<app-port>` or `127.0.0.1:<app-port>`, Lima's `hostagent` automatically forwards that port to macOS `127.0.0.1:<app-port>` ([Lima: Port Forwarding](https://lima-vm.io/docs/config/port/)), so the host Chrome instance can load `http://127.0.0.1:<app-port>/` directly.

### 2.2 Isolation and test-plumbing trade-offs in `thirdshift`

| Execution context | Can reach `http://192.168.5.2:<port>`? | Why |
| --- | --- | --- |
| **Sandboxed Trials and model Graders** ([`src/sandbox.rs` L502-523, L688-695](../../src/sandbox.rs#L502-L523)) | **No (`ENETUNREACH` / proxy refusal)** | `bwrap` runs with `--unshare-net`, and the allowlisting CONNECT proxy ([`src/proxy.rs` L159-187](../../src/proxy.rs#L159-L187)) permits only the **Harness**'s model-API hostnames while refusing IP literals, loopback, and local host addresses. |
| **Sandboxed code Graders** ([`src/sandbox/code_grader.rs` L277-313](../../src/sandbox/code_grader.rs#L277-L313)) | **No (`ENETUNREACH`)** | `bwrap` runs with `--unshare-net` and no proxy relay. |
| **Unsandboxed Commands** (**Ticket run**, **Spec run**, **Review run**, **Fix run**; [`docs/adr/0013-every-harness-runs-unattended-and-fully-trusted.md`](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md)) and `--read-machine` **Triage runs** ([`src/sandbox/triage.rs` L17-61](../../src/sandbox/triage.rs#L17-L61)) | **Yes** | Sessions keep VM TCP networking. Inside a `--mount-none` VM, exposing a host Chrome CDP socket gives guest sessions control of a browser process executing as the macOS host user (capable of navigating `file://` URLs or macOS `127.0.0.1` services unless further sandboxed on macOS). |

In addition to weakening the `--mount-none` VM boundary documented in [`docs/research/linux-on-macbook.md`](linux-on-macbook.md#15-filesystem-placement-and-git-spawn-performance), standard repository Playwright suites invoke `chromium.launch(...)` rather than checking for a remote CDP endpoint, so agent-authored browser tests will still spawn guest Chromium unless the target repository explicitly wires `connectOverCDP`.

---

## 3. Three.js WebGL2 Benchmarks and Pixel Fidelity (Apple M5 Pro)

To quantify the gap between native Metal and Linux VM CPU rasterizers, we timed two Three.js `WebGLRenderer` (`ACESFilmicToneMapping`, `PCFSoftShadowMap`, `antialias: true`) scenes after shader compilation warmup, synchronizing each frame with a 4×4 `gl.readPixels` fence so GPU command buffers retired before recording frame latency:

1. **Maquette (`1280×720`, 30 timed frames after 5 warmup frames):** ~38,000 triangles, `1024×1024` directional shadow map, PBR `InstancedMesh` terrain/buildings/units, emissive windows, and alpha-blended sightline cones.
2. **Heavy (`1920×1080`, 15 timed frames after 5 warmup frames):** ~581,000 triangles (400 high-segment torus-knot PBR instances + 250 shadow-casting columns), `2048×2048` directional shadow map, hemisphere + point + directional lighting.

| Environment & WebGL2 renderer | Maquette (`1280×720`, ~38k tris) | 720p `page.screenshot()` | Heavy (`1920×1080`, ~581k tris) | 1080p `page.screenshot()` |
| --- | --- | --- | --- | --- |
| **macOS Chrome over CDP (`ANGLE Metal: Apple M5 Pro`)** | **1.31 ms/frame** (763.4 fps, p50 1.2 ms) | 78.0 ms | **3.67 ms/frame** (272.2 fps, p50 3.7 ms) | 373.7 ms |
| **Linux VM `SwiftShader` (6 M5 Pro vCPUs)** | **84.62 ms/frame** (11.8 fps, p50 87.6 ms) | 62.0 ms | **562.05 ms/frame** (1.8 fps, p50 525.9 ms) | 262.7 ms |
| **Linux VM `Mesa llvmpipe` (LLVM 22.1, 6 M5 Pro vCPUs)** | **87.95 ms/frame** (11.4 fps, p50 84.0 ms) | 92.0 ms | **587.77 ms/frame** (1.7 fps, p50 571.6 ms) | 302.0 ms |

### 3.1 Visual parity and impact on **Spec** / **Ticket** output quality

- **Pixel and shader parity:** Center-canvas RGBA samples across `ANGLE Metal`, `SwiftShader`, and `Mesa llvmpipe` matched within **`±1/255`** with `gl.getError() === 0`. Because both `ANGLE/Metal` and `ANGLE/SwiftShader` validate and translate GLSL ES 3.00 through Google's `ANGLE` frontend, WebGL2 features such as shadow maps, MSAA, 8-bit stencil silhouette passes (`EqualStencilFunc` / `GreaterDepth`), and custom `ShaderMaterial` fragment math behave identically.
- **Static Before/After PR captures:** When [`skills/thirdshift-implement/SKILL.md`](../../skills/thirdshift-implement/SKILL.md) and [`skills/thirdshift-pr/SKILL.md`](../../skills/thirdshift-pr/SKILL.md) capture `1440×900` `.png` screenshots for PR visual evidence or agent self-inspection, `SwiftShader` produces visually indistinguishable images in ~85–150 ms.
- **When hardware GPU rendering genuinely helps:**
  1. **Hardware `<16.6 ms` (60 fps) frame-budget assertions** (impossible on ~85 ms/frame CPU rasterizers, though draw-call and allocation regressions can still be asserted deterministically via `renderer.info.render.calls` and `renderer.info.memory.geometries`).
  2. **Real-time 60 fps `.gif` or video recordings** when an animation loop uses wall-clock `requestAnimationFrame` rather than deterministic per-frame stepping.
  3. **Heavy 3D or WebGPU workloads** (500k+ triangle scenes, multi-pass post-processing such as SSAO/bloom, or Three.js `WebGPURenderer` compute shaders) where CPU rasterization drops below 2 fps.

---

## 4. Linux `bwrap` Sandbox GPU Passthrough (Native Linux and Cloud GPU Hosts)

On a bare-metal or cloud Linux GPU host (for example, an `x86_64` Ubuntu server with an NVIDIA or AMD GPU), native Vulkan drivers (`nvidia`, `radv`, `anv`) expose `VK_KHR_swapchain` and `VK_KHR_external_semaphore_fd` directly, avoiding every MoltenVK/Venus limitation in §1.2.

In `thirdshift`:

- **Unsandboxed Commands** (**Ticket runs**, **Spec runs**, **Review runs**, **Fix runs**) run directly on the Linux host under [`docs/adr/0013-every-harness-runs-unattended-and-fully-trusted.md`](../adr/0013-every-harness-runs-unattended-and-fully-trusted.md) and already inherit `/dev/dri` and `/sys` with no changes.
- **Sandboxed Trials and code Graders** call `base_argv()` ([`src/sandbox.rs` L688-723](../../src/sandbox.rs#L688-L723), [`src/sandbox/code_grader.rs` L277-313](../../src/sandbox/code_grader.rs#L277-L313)), which mounts a fresh `--dev /dev` tmpfs (containing only basic pseudo-devices like `null`, `zero`, `urandom`, `tty`) and does **not** bind `/sys`.

Testing four `bwrap` configurations against `/dev/dri/renderD128` revealed that binding `/dev/dri` alone is insufficient:

| `bwrap` device & sysfs flags | `vulkaninfo --summary` result | Reason |
| --- | --- | --- |
| Current `base_argv()`: `--dev /dev` (no `/sys`) | **Fails** (`ERROR_INITIALIZATION_FAILED`) | `/dev/dri` does not exist inside the sandbox's private `/dev`. |
| `+ --dev-bind-try /dev/dri /dev/dri` (no `/sys`) | **Fails** (`ERROR_INITIALIZATION_FAILED`) | Mesa's Vulkan loader enumerates DRM render nodes through `libdrm`'s `drmGetDevices2()` ([`xf86drm.c`](https://gitlab.freedesktop.org/mesa/drm/-/blob/main/xf86drm.c)), which resolves `/sys/dev/char/<major>:<minor>/device` under `/sys/devices`. Without those `/sys` paths, `drmGetDevices2()` returns 0 devices even when `/dev/dri/renderD128` is openable. |
| `+ --dev-bind-try /dev/dri /dev/dri --ro-bind /sys/dev/char /sys/dev/char --ro-bind /sys/devices /sys/devices` | **Passes** (`GPU0` detected) | Satisfies both `/dev/dri/renderD128` ioctl access and `libdrm` PCI/virtio sysfs device enumeration while keeping sysfs read-only. |
| Same binds `+ --unshare-user --cap-drop ALL` (matching [`src/sandbox/triage.rs` L46-59](../../src/sandbox/triage.rs#L46-L59)) | **Passes** (`GPU0` detected) | Unprivileged user namespaces and dropped capabilities do not block render-node ioctls when the host user has read/write permissions on `/dev/dri/renderD128` (`0666` or `render` group). |

**NVIDIA host note:** On Linux hosts using NVIDIA's proprietary/open-kernel-module Vulkan ICD (`nvidia_icd.json`), userland also opens `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-modeset`, and `/dev/nvidia-uvm` alongside `/dev/dri`, so a future GPU-passthrough option for **Trials** and code **Graders** must `--dev-bind-try` any present `/dev/nvidia*` character devices in addition to `/dev/dri`, `/sys/dev/char`, and `/sys/devices`.

---

## 5. Summary Recommendations

| Workload | Recommended environment | Notes |
| --- | --- | --- |
| **Ordinary Three.js / WebGL2 smoke suites & static `1440×900` PR screenshots** | Standard Linux host or Lima `vz` VM (`--mount-none`) using Chromium `SwiftShader` (`--use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader`) | Zero isolation compromises, `±1/255` pixel parity with Metal, ~85 ms/frame at 720p on Apple M5 Pro. |
| **Vulkan compute inside a macOS Linux VM (`llama.cpp`, `vulkaninfo`)** | Lima `krunkit` VM (Fedora + `slp/mesa-libkrun-vulkan` 16 KiB-aligned Mesa) | Exposes `Virtio-GPU Venus (Apple M5 Pro)` for compute; does **not** accelerate Chromium WebGL2/WebGPU. |
| **Hardware 60 fps browser rendering, heavy PBR/WebGPU scenes, or `<16.6 ms` frame-budget checks** | Native Linux GPU host (cloud NVIDIA/AMD server; add `/dev/dri` + `/dev/nvidia*` + read-only `/sys/dev/char` and `/sys/devices` if needed inside `bwrap` **Trials** / **Graders**) | Full Vulkan WSI/swapchain support inside Linux without breaking macOS VM isolation; reserve macOS host-CDP (`connectOverCDP`) for manual local prototyping unless explicitly governed by an ADR. |
