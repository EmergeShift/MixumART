# Patched CRIU 4.1 for mixumOS

`criu-mixum` is shipped as a source package in mixumOS
(`dist/pkgs/criu/PKGBUILD`) and is a dependency of `mixum-base`, so normal
images get the patched `/usr/bin/criu`. The manual build below remains useful
for non-mixumOS hosts.

The stock Arch `criu` 4.2.1 crashes in the parasite on the 6.18 mixum kernel
(`si_status=11`). Build 4.1 locally:

```bash
pacman -S --needed base-devel git libnet protobuf-c libnl
curl -L -o criu-4.1.tar.gz \
  https://github.com/checkpoint-restore/criu/archive/refs/tags/v4.1.tar.gz
tar xf criu-4.1.tar.gz
cd criu-4.1
```

Apply this compatibility edit to `criu/include/linux/rseq.h`: remove the
`#ifdef CONFIG_HAS_NO_LIBC_RSEQ_DEFS ... #endif` block. Current glibc already
defines those enums; keeping them fails with duplicate-definition errors.

Then:

```bash
make -j"$(nproc)" WERROR=0
install -m755 criu/criu /usr/bin/criu
rm -rf /usr/lib/criu   # do not load 4.2.1 plugins with 4.1
```

The container needs the opt-in capabilities and a writable sysctl namespace:

```bash
mixum env create --title mem --image mixumos:0.1.5 --memory-snapshots
```

That adds:

```text
SYS_ADMIN SYS_PTRACE CHECKPOINT_RESTORE SYS_RESOURCE NET_ADMIN SYS_TIME
```

and exports `MIXUM_MEMORY_SNAPSHOTS=1`; agentd remounts `/proc/sys` read-write.

## Current limits

`criu check --all` still reports dirty tracking OFF. On arm64 this is
architectural: `CONFIG_MEM_SOFT_DIRTY` depends on `HAVE_ARCH_SOFT_DIRTY`, which
arm64 does not select (x86 does), so the config cannot be enabled here. A
simple process dump/restore works, but incremental dirty-page tracking needs a
custom implementation; do not claim production incremental snapshots on arm64. The earlier mixum-task parasite
hang was caused by agentd SIGSTOP-freezing the task before CRIU; memory-only
snapshots now skip that pre-freeze. Disk/hybrid still freeze for file
consistency.

A memory-only snapshot captures only process memory and restores into the
existing disk state; use hybrid if both tree and memory must move together.

Verified agentd path: memory snapshot created 15 chunks, `snap.restore` returned
success, and the `sleep` task reappeared in the container. The sandbox cgroup is
kept for memory environments and thawed before CRIU restore.

The patched binary can also be selected explicitly:

```bash
MIXUM_CRIU_BIN=/usr/local/bin/criu41
```
