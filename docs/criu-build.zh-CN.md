# 为 mixumOS 打过补丁的 CRIU 4.1

`criu-mixum` 以源码包形式随 mixumOS 发布（`dist/pkgs/criu/PKGBUILD`），并且是
`mixum-base` 的依赖，所以正常镜像自带打过补丁的 `/usr/bin/criu`。下面的手动构建
仍然适用于非 mixumOS 宿主。

Arch 自带的 `criu` 4.2.1 在 6.18 mixum 内核上会于 parasite 阶段崩溃
（`si_status=11`）。请本地构建 4.1：

```bash
pacman -S --needed base-devel git libnet protobuf-c libnl
curl -L -o criu-4.1.tar.gz \
  https://github.com/checkpoint-restore/criu/archive/refs/tags/v4.1.tar.gz
tar xf criu-4.1.tar.gz
cd criu-4.1
```

对 `criu/include/linux/rseq.h` 应用这个兼容性修改：删掉
`#ifdef CONFIG_HAS_NO_LIBC_RSEQ_DEFS ... #endif` 整块。当前 glibc 已经定义了这些
enum；保留它们会因重复定义而编译失败。

然后：

```bash
make -j"$(nproc)" WERROR=0
install -m755 criu/criu /usr/bin/criu
rm -rf /usr/lib/criu   # 不要让 4.1 加载 4.2.1 的插件
```

容器需要显式选择的能力和一个可写的 sysctl namespace：

```bash
mixum env create --title mem --image mixumos:0.1.5 --memory-snapshots
```

它会加上：

```text
SYS_ADMIN SYS_PTRACE CHECKPOINT_RESTORE SYS_RESOURCE NET_ADMIN SYS_TIME
```

并导出 `MIXUM_MEMORY_SNAPSHOTS=1`；agentd 会把 `/proc/sys` remount 为可写。

## 当前限制

`criu check --all` 仍会报告 dirty tracking OFF。在 arm64 上这是架构限制：
`CONFIG_MEM_SOFT_DIRTY` 依赖 `HAVE_ARCH_SOFT_DIRTY`，而 arm64 没有选择它（x86 有），
所以这个配置在这里无法开启。简单的进程 dump/restore 可用，但增量脏页跟踪需要自定义
实现；不要在 arm64 上声称生产级增量快照。之前 mixum-task 的 parasite 挂起，是 agentd
在 CRIU 之前用 SIGSTOP 冻结任务导致的；纯内存快照现在会跳过这次前置冻结。
disk/hybrid 仍然会冻结，以保证文件一致性。

纯内存快照只捕获进程内存，并恢复到已有的磁盘状态；如果树和内存必须一起迁移，请用
hybrid。

已验证的 agentd 路径：内存快照创建了 15 个 chunk，`snap.restore` 返回成功，`sleep`
任务重新出现在容器里。内存环境会保留 sandbox cgroup，并在 CRIU restore 前解冻。

也可以显式选择打过补丁的二进制：

```bash
MIXUM_CRIU_BIN=/usr/local/bin/criu41
```
