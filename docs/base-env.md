# Base environments (母体环境)

A base is an immutable OCI image built from a human-controlled root builder. It is
shared by many environments; each environment gets its own writable named volumes
for `/var/lib/mixum/work` and `/var/lib/mixum/home`.

## Build

```bash
mixum base create node22 --image mixumos:0.1.5
mixum base shell node22          # root shell, human only
# pacman -S chromium nodejs npm ...
mixum base commit node22 --tag v1
mixum env create --title api --base node22:v1
```

`base commit` exports the builder rootfs as a tar (GNU tar, xattrs/acls preserved),
generates a `FROM scratch` + `ADD rootfs.tar /` Containerfile carrying the source
image's ENV/ENTRYPOINT/CMD/WORKDIR/USER, and builds it with Apple Container's own
builder. No Docker, no registry, and no per-container recompilation.

Excluded from the image: `/proc`, `/sys`, `/dev`, `/run`, `/tmp`,
`/var/lib/mixum`, `/etc/machine-id`, SSH host keys.

## Switch

```bash
mixum env rebase api --base node22:v2
```

`env rebase` stops the old container (releasing Apple Container's volume lock),
creates a new container from the new base with the same named volumes, starts it,
and keeps the old container stopped as a rollback target. Workspace and home are
preserved; no image is rebuilt during the switch.

Rebase requires `--base` environments with persistent volumes. A lossy in-place
swap is refused.

## Platforms

- **Apple Container**: native rootfs tar → `container build` (path above).
- **Docker**: same rootfs tar → `docker build`.
- **WSL 2**: builder distro + `wsl --export` rootfs tar; the tar is the image.
  WSL2 has no named-volume mount in this CLI yet, so rebase is not supported there.
- **Linux bare metal**: use the Docker backend. KVM/microVM isolation is a
  separate runtime, not a base-image concern.
