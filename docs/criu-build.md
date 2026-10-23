# Patched CRIU 4.1 for mixumOS

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

`criu check --all` still reports dirty tracking OFF. Enable
`CONFIG_MEM_SOFT_DIRTY` in the custom kernel before claiming production memory
snapshots. A simple process dump/restore works, but a task started by the mixum
Landlock/setuid sandbox was observed to hang in parasite injection; do not
advertise memory snapshots for agent tasks until that is fixed.

The patched binary can also be selected explicitly:

```bash
MIXUM_CRIU_BIN=/usr/local/bin/criu41
```
