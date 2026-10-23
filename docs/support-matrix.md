> English translation of the Chinese document. Original: [support-matrix.zh-CN.md](support-matrix.zh-CN.md). Technical review pending.

# Host support matrix

This page is **MixumART's boundary statement**: what each host can do, how far it
goes, and what is physically impossible. Product copy, UI disabling logic and issue
triage all follow this table; change this page before changing a capability.

Legend:

| Mark | Meaning |
|---|---|
| ✅ | implemented and verified |
| 🟡 | code ready, not verified on real hardware / has known limits |
| 🧪 | needs a specific kernel or privilege; PoC/experimental |
| ❌ | not supported physically or by product decision |
| — | not applicable to that host |

## 1. Host × capability

| Capability | macOS<br>Apple Container | macOS<br>Docker Desktop | Linux<br>Docker Engine | Windows<br>WSL 2 | Windows<br>Docker Desktop | Linux<br>QEMU microVM |
|---|---|---|---|---|---|---|
| Container lifecycle (create/start/stop/delete) | ✅ | ✅ | ✅ | ✅ | ✅ | 🧪 |
| Container logs / persistent terminal | ✅ | ✅ | 🟡 | ✅ | 🟡 | 🧪 |
| Image import / pull | ✅ | ✅ | ✅ | ✅ | ✅ | 🧪 |
| Disk snapshots (create/tree/rollback/fork) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Memory snapshots (CRIU dump) | ✅ | ❌ | 🧪 | 🧪 | ❌ | 🧪 |
| Same-host memory rollback (CRIU restore) | ✅ | ❌ | 🧪 | 🧪 | ❌ | 🧪 |
| Cross-device `.mart` migration (files) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Cross-device migration (process memory) | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| GUI stream (mixcomp A) | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| GUI stream (B compositor) | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| Native Metal window | ✅ | ✅ | — | ❌ | ❌ | — |
| Input / clipboard / IME | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| XWayland | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| Landlock file allowlist | 🧪 | ❌ | ✅ | 🧪 | ❌ | 🧪 |
| cgroup v2 freezer | 🟡 | 🟡 | ✅ | 🟡 | 🟡 | ✅ |
| FUSE lazy mount | 🧪 | 🧪 | 🧪 | 🧪 | 🧪 | 🧪 |
| Custom guest kernel | ✅ | ❌ | ❌ | ✅ | ❌ | ✅ |

> Whether GUI streaming works does not depend on the host; it depends on
> `mixum-agentd` + `mixcomp` (or the B compositor) running in the container. The host
> only provides the MAP/WSS connection and window rendering. Windows is 🟡 because the
> full window path has not been verified on this machine.

## 2. Per-host notes

### 2.1 macOS + Apple Container

- **Runtime**: `container` CLI 1.5+, one lightweight Linux VM per container.
- **Kernel**: kata by default; `container system kernel set --binary` can switch to
  `linux-mixum-micro`.
- **CRIU**: the default kernel has no `CHECKPOINT_RESTORE`; a custom kernel is
  required. CRIU also needs privileges inside the container (`CAP_SYS_ADMIN`/
  `SYS_PTRACE`); whether Apple Container allows them is still a PoC question.
  (2026-10-22, Apple Container + `linux-mixum-micro`: dump the whole Sway+foot session
  tree → kill → restore, `swaymsg -t get_tree` still shows `app_id: foot`; the mixcomp
  window tree needs a whole-container dump and is still being wired. CRIU 3.17.1
  works; 4.2.1 SIGSEGVs in parasite on this kernel.)
- **Kernel is system-wide**: switching affects every Apple Container on the machine;
  install/rollback scripts are mandatory.
- **Disk snapshots**: available; windows replay from `guiSessions` after restore
  (Task 5).
- **Migration**: `.mart` file layer crosses devices; memory does not.
- **Native Metal**: MixumART renders PNG frames through an MTKView native overlay on
  macOS, ✅.

### 2.2 macOS + Docker Desktop

- **Kernel fixed**: Docker Desktop's LinuxKit VM kernel cannot be replaced; no CRIU.
- **Disk snapshots / `.mart` migration**: available.
- **Landlock**: the Docker Desktop VM kernel does not enable it in practice; the task
  layer's Landlock does not take effect (falls back to the container boundary).
- **Docker Desktop is only a "can run it" minimum**, not a full-capability host.

### 2.3 Linux + Docker Engine

- **Kernel = host kernel**: CRIU, Landlock, cgroup and FUSE depend on host config and
  container privileges.
- With host kernel ≥5.13 and `CONFIG_SECURITY_LANDLOCK`, Task 4's Landlock works.
- CRIU needs a privileged container + `CHECKPOINT_RESTORE`; the same-kernel constraint
  still applies.
- This is the **most complete and controllable** host; good CI/PoC baseline.

### 2.4 Windows + WSL 2

- **Kernel replaceable**: point `[wsl2] kernel=` in `.wslconfig` at
  `linux-mixum-micro`.
- WSL2 driver requirements (Hyper-V, 9p, vsock) must also be met; the
  `linux-mixum-micro` config already includes them.
- CRIU needs root inside WSL2; the same-kernel constraint applies.
- GUI: Tauri renders through a WebView canvas on Windows, without native textures,
  hence 🟡.

### 2.5 Windows + Docker Desktop

- Same class as macOS Docker Desktop: the kernel is managed by Docker Desktop and
  cannot be replaced; CRIU ❌.
- Other file-layer capabilities work.

### 2.6 Linux + QEMU microVM

- Boot `linux-mixum-micro` directly as a microVM; the shape closest to Apple
  Container.
- MixumART has no QEMU runtime backend yet, hence 🧪; it needs a new backend before
  it can be productized.
- This is the reference path for "no hypervisor vendor lock-in".

## 3. Kernel matrix

| | `linux-mixum-micro` | `linux-mixum` |
|---|---|---|
| Base | kata 6.18.35 + mixum fragments | Arch kernel config + mixum fragments |
| Target | Apple Container / WSL2 / QEMU microVM guest | full disk image / general Linux VM / bare metal |
| Boot | sub-second | 2–5 s |
| Resident | small (30–80 MB less per instance) | full distro kernel |
| CRIU | ✅ compiled in (needs privilege) | ✅ compiled in |
| Landlock / FUSE | ✅ compiled in | ✅ compiled in |
| Hardware coverage | narrow (mostly virtio/Hyper-V) | wide (distro drivers/modules/firmware) |
| Install | `container system kernel set` / `.wslconfig` | pacman package / image |
| Version fingerprint | `uname -r` contains `-mixum-micro` | `uname -r` contains `-mixum` |

> Both kernels share the same config-fragment source; **memory snapshots require an
> exact node kernel fingerprint**, so a kernel upgrade invalidates old memory/hybrid
> nodes (disk nodes already allow cross-kernel).

## 4. Cross-device migration matrix

The migration unit is **image digest + container spec + `.mart` snapshot + node
metadata (including guiSessions)**. It is not "moving a live container".

| Source → target | Disk snapshot | Memory snapshot | GUI windows | Notes |
|---|---|---|---|---|
| Same host, same runtime | ✅ | 🧪 | ✅ replay | CRIU only same kernel |
| WSL2 → Apple Container (same arm64) | ✅ | ❌ | ✅ replay | same arch, same image |
| WSL2 → Linux Docker (same arch) | ✅ | ❌ | ✅ replay | target image must be pullable |
| Apple Container → Linux Docker (same arm64) | ✅ | ❌ | ✅ replay | same as above |
| x86_64 → arm64 | ❌ | ❌ | — | binaries incompatible; migrate source and rebuild |
| Docker Desktop → any | ✅ | ❌ | ✅ replay | no CRIU on the source |
| Any → QEMU microVM | ✅ | ❌ | ✅ replay | needs the QEMU backend |

**What is not migrated across devices**: running processes, terminal scrollback,
packages installed with `pacman -S` inside the container, `/etc` changes,
tokens/certificates, port mappings, unsaved editor buffers.

## 5. Physical/product boundaries (not done)

1. **CRIU across hypervisors**: WSL2 (Hyper-V) ↔ Apple Container (virtio) ↔ Docker
   Desktop (another virtio topology) — even with the same kernel version, device fds
   and virtio state do not line up.
2. **Cross-architecture memory migration**: there is no general process-memory
   migration between x86_64 and arm64.
3. **Docker Desktop custom kernel**: the kernel is managed by Docker Desktop and
   cannot be replaced from user space.
4. **In-place Wayland session restore**: there is no standard state serialization;
   cross-device only supports app restart + window replay.
5. **Unprivileged CRIU**: CRIU needs `CAP_SYS_ADMIN`/`SYS_PTRACE`; without a
   privileged variant, memory snapshots are not offered.

## 6. Host acceptance checklist

Before claiming "supported" on any host, run at least:

```sh
# 1. Runtime
<runtime> version && <runtime> ps

# 2. agentd / mixcomp in the container
mixumd --config /dev/null &            # watch for READY
mixcomp --sock /run/mixum/display.sock &

# 3. Disk snapshot
#    GUI: create → change a file → restore → file rolls back; windows reopen from guiSessions

# 4. Memory snapshot (🧪 hosts only)
criu check && criu dump --tree <pid> --images-dir /tmp/d --shell-job
criu restore --images-dir /tmp/d --shell-job

# 5. Migration
#    export .mart → import on target → start the same image → restore → replay windows
```

## 7. Relation to other docs

- Runtime contract: [`runtime-contract.md`](runtime-contract.md)
- Display architecture: [`display.md`](display.md)
- Kernel build and acceptance: `mixumos/kernel/README.md`
- Migration bundle format: `mixumos/docs/migration.md` (to be completed)
