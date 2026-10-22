# Snapshot limits (verified 2026-10-23)

## Apple Container

Apple Container's default container has no `CAP_SYS_ADMIN` or
`CAP_CHECKPOINT_RESTORE`, and `/proc/sys` is read-only. `criu check --all`
therefore fails at `sys/kernel/ns_last_pid` even when CRIU itself is present.

Consequences:

- **disk** snapshots work (files + `.mart`) and are the supported tier here;
- **memory/hybrid** snapshots are not production-ready on Apple Container;
- adding `--cap-add ALL` gets CRIU further, but cannot make `/proc/sys` writable;
- the correct long-term fix is the VM runner: run mixumOS in a microVM with its
  own kernel and a writable `/proc/sys`, then CRIU inside that VM.

## CRIU build notes

- Arch package 4.2.1 has a parasite SIGSEGV on this kernel.
- Building 3.17.1 fails against current glibc headers (`fsconfig`/`fsmount`).
- Building 4.1 from source required removing duplicate rseq enums and building
  with `WERROR=0`; the resulting binary still failed `criu check` in Apple
  Container for the permission/procfs reasons above.
- Do not advertise memory snapshots until the VM runner or a privileged Linux
  host is the target.

## Cross-arch

Memory images remain architecture/kernel specific. `.mart` disk bundles remain
portable; `memory`/`hybrid` restore must be rejected across arch or kernel.
