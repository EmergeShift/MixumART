# 快照限制（2026-10-23 验证）

## Apple Container

自定义内核是必要条件，但不是充分条件。我们启动的内核有：

```text
CONFIG_CHECKPOINT_RESTORE=y
CONFIG_PID_NS=y
CONFIG_USER_NS=y
```

默认的 Apple Container 进程仍然拿到空 capability 集和只读的 `/proc/sys`：

```text
CapEff: 00000000a80425fb
proc on /proc/sys type proc (ro,relatime)
```

`criu check --all` 在那里失败，本文第一版因此错误地得出“Apple Container 永远不能
checkpoint”的结论。这个结论太宽了。

显式加上 capability 集和可写 sysctl 挂载后：

```text
--cap-add SYS_ADMIN
--cap-add SYS_PTRACE
--cap-add CHECKPOINT_RESTORE
--cap-add SYS_RESOURCE
--cap-add NET_ADMIN
--cap-add SYS_TIME
mount -o remount,rw /proc/sys
```

打过补丁的 CRIU 4.1 在同一个自定义内核上完成了两步：

```text
criu dump   -> 0
criu restore-> 0
```

agentd 路径现在也验证过：对一个沙箱里的 `sleep` 创建内存快照（15 个 chunk），
`snap.restore` 返回成功，恢复后的任务回到容器里。需要两个重要细节：

- 纯内存快照跳过宿主的 SIGSTOP 冻结（CRIU 自己会冻结）；
- 内存快照环境保留 sandbox cgroup 目录，restore 在 CRIU 把任务放回之前先解冻。

GUI 环境（agent task + foot + mixcomp）也完成了内存快照与恢复；恢复后合成器和 foot
都回来了。

Hybrid 快照现在在宿主冻结下打包磁盘树，然后在 CRIU dump 内存前解冻。目标镜像上
真实的 hybrid create/restore 返回成功，并恢复了 `sleep` 任务。

所以真正的要求是：

1. 带 `CONFIG_CHECKPOINT_RESTORE`（以及 PID/user namespace）的自定义内核；
2. 显式 capability，而不是默认容器 capability 集；
3. `/proc/sys` 重新挂载为读写；
4. 能在这个内核上工作的 CRIU 版本。

## CRIU 版本

- Arch 包 4.2.1：dump 时 parasite SIGSEGV（`si_status=11`）。
- 3.17.1：对当前 glibc 头文件编译失败。
- 4.1：删掉重复的 rseq 定义并加 `WERROR=0` 后可编译；简单进程的 dump/restore
  在这个内核上可用。
- 不能把内置 `criu` 包的插件和打过补丁的二进制混用；把 `/usr/lib/criu` 移开，
  或设置插件路径。

## Dirty tracking

`criu check --all` 仍会警告：

```text
Dirty tracking is OFF. Memory snapshot will not work.
```

这**不是** Apple Silicon 上缺了内核 fragment。`CONFIG_MEM_SOFT_DIRTY` 依赖
`HAVE_ARCH_SOFT_DIRTY`；arm64/Linux 没有选择这个符号（x86 有）。加这个配置会静默
失败于依赖，所以内核构建从未暴露它。简单任务的一次性 dump/restore 可用，但在 arm64
上没有自定义页跟踪实现就没有 pre-dump/增量脏页跟踪。不要在这个架构上声称生产级
增量内存快照。

第一个 agent 任务 dump 挂起，是因为 agentd 在 CRIU 之前用 SIGSTOP 预冻结了任务。
CRIU 自己会冻结整棵树；纯内存快照现在跳过宿主冻结钩子（disk/hybrid 仍会冻结，
以保证文件树一致）。

## 安全权衡

`mixum env create --memory-snapshots` 会加上那些 capability 并导出
`MIXUM_MEMORY_SNAPSHOTS=1`；agentd 在启动时重新挂载 `/proc/sys`。这些 capability
是容器级的。普通 agent 任务仍以 `mixum` 运行，用不到它们；但被攻破的 root agentd
会获得大得多的攻击面。因此内存快照应当按环境显式 opt-in，而不是默认开启。VM
runner 仍是更干净的隔离边界。

## 跨架构

内存镜像仍然绑定架构/内核。`.mart` 磁盘包仍可移植；`memory`/`hybrid` 恢复必须在
跨架构或跨内核时拒绝。
