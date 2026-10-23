# Snapshot limits (verified 2026-10-23)

## Apple Container

A custom kernel is necessary but not sufficient. The kernel we booted had:

```text
CONFIG_CHECKPOINT_RESTORE=y
CONFIG_PID_NS=y
CONFIG_USER_NS=y
```

The default Apple Container process still gets an empty capability set and a
read-only `/proc/sys`:

```text
CapEff: 00000000a80425fb
proc on /proc/sys type proc (ro,relatime)
```

`criu check --all` fails there, and the first version of this document wrongly
concluded that Apple Container could never checkpoint. That conclusion was too
broad.

With an explicit capability set and a writable sysctl mount:

```text
--cap-add SYS_ADMIN
--cap-add SYS_PTRACE
--cap-add CHECKPOINT_RESTORE
--cap-add SYS_RESOURCE
--cap-add NET_ADMIN
--cap-add SYS_TIME
mount -o remount,rw /proc/sys
```

a patched CRIU 4.1 completed both operations on the same custom kernel:

```text
criu dump   -> 0
criu restore-> 0
```

The agentd path is now also verified: a memory snapshot of a sandboxed `sleep`
was created (15 chunks), and `snap.restore` returned success with the restored
task back in the container. Two important details were required:

- memory-only snapshots skip the host SIGSTOP freeze (CRIU freezes itself);
- memory-snapshot environments keep the sandbox cgroup directory, and restore
  thaws it before CRIU moves the task back in.

So the real requirements are:

1. custom kernel with `CONFIG_CHECKPOINT_RESTORE` (and PID/user namespaces);
2. explicit capabilities, not the default container capability set;
3. `/proc/sys` remounted read-write;
4. CRIU version that works on this kernel.

## CRIU version

- Arch package 4.2.1: parasite SIGSEGV (`si_status=11`) on dump.
- 3.17.1: fails to compile against current glibc headers.
- 4.1: builds after removing duplicate rseq definitions and with `WERROR=0`;
  dump/restore of a simple process works on this kernel.
- The built-in `criu` package's plugins must not be loaded with the patched
  binary; move `/usr/lib/criu` aside or set the plugin path.

## Dirty tracking

`criu check --all` still warns:

```text
Dirty tracking is OFF. Memory snapshot will not work.
```

The current kernel config does not expose soft-dirty tracking. One-shot
dump/restore of a simple task now works, but pre-dump/dirty-page handling is not
safe until `CONFIG_MEM_SOFT_DIRTY` is enabled and checked. Do not claim
production memory snapshots before that check passes.

The first agent task dump hung because agentd pre-froze the task with SIGSTOP
before CRIU. CRIU freezes the tree itself; memory-only snapshots now skip the
host freeze hook (disk/hybrid still freeze for a consistent file tree).

## Security tradeoff

`mixum env create --memory-snapshots` adds those capabilities and exports
`MIXUM_MEMORY_SNAPSHOTS=1`; agentd remounts `/proc/sys` at startup. The
capabilities are container-wide. Normal agent tasks still run as `mixum`
and cannot use them, but a compromised root agentd gets a much larger attack
surface. Memory snapshots should therefore be an explicit opt-in per
environment, not a default. A VM runner remains the cleaner isolation boundary.

## Cross-arch

Memory images remain architecture/kernel specific. `.mart` disk bundles remain
portable; `memory`/`hybrid` restore must be rejected across arch or kernel.
