# 母体环境（base environments）

母体环境是一个不可变的 OCI 镜像，由人类控制的 root builder 构建。它被多个环境共享；
每个环境拥有自己的可写命名卷，用于 `/var/lib/mixum/work` 和 `/var/lib/mixum/home`。

## 构建

```bash
mixum base create node22 --image mixumos:0.1.5
mixum base shell node22          # root shell，仅限人类
# pacman -S chromium nodejs npm ...
mixum base commit node22 --tag v1
mixum env create --title api --base node22:v1
```

`base commit` 把 builder 的 rootfs 导出为 tar（GNU tar，保留 xattrs/acls），生成一个
`FROM scratch` + `ADD rootfs.tar /` 的 Containerfile，继承源镜像的
ENV/ENTRYPOINT/CMD/WORKDIR/USER，再用 Apple Container 自己的 builder 构建。不需要
Docker、不需要 registry，也不按容器重新编译。

镜像中排除：`/proc`、`/sys`、`/dev`、`/run`、`/tmp`、`/var/lib/mixum`、
`/etc/machine-id`、SSH host keys。

## 切换

```bash
mixum env rebase api --base node22:v2
```

`env rebase` 停掉旧容器（释放 Apple Container 的卷锁），用新母体创建同名的新容器、
挂同样的命名卷、启动，并把旧容器保留为已停止的回滚目标。工作区与 home 都会保留；
切换过程中不重建镜像。

Rebase 只支持带持久卷的 `--base` 环境；有损的原地替换会被拒绝。

## 平台

- **Apple Container**：原生 rootfs tar → `container build`（上面的路径）。
- **Docker**：同一个 rootfs tar → `docker build`。
- **WSL 2**：builder 发行版 + `wsl --export` rootfs tar；这个 tar 就是镜像。
  当前 CLI 在 WSL2 上还没有命名卷挂载，因此不支持 rebase。
- **Linux 裸机**：使用 Docker 后端。KVM/microVM 隔离是另一种运行时，不是母体镜像要
  解决的问题。
