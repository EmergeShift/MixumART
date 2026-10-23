# 宿主支持矩阵（Support Matrix）

本文是 **MixumART 的边界声明**：每个宿主能做什么、做到什么程度、哪些是物理上做不到的。
产品文案、UI 灰化逻辑、issue 分诊都以这张表为准；改动能力前先改这里。

状态图例：

| 标记 | 含义 |
|------|------|
| ✅ | 已实现并验证 |
| 🟡 | 代码就绪，未在真机验证 / 有已知限制 |
| 🧪 | 需要特定内核或特权，属于 PoC/实验能力 |
| ❌ | 物理或产品上不支持，不做 |
| — | 该宿主不适用 |

## 1. 宿主 × 能力

| 能力 | macOS<br>Apple Container | macOS<br>Docker Desktop | Linux<br>Docker Engine | Windows<br>WSL 2 | Windows<br>Docker Desktop | Linux<br>QEMU microVM |
|------|--------------------------|--------------------------|------------------------|------------------|---------------------------|------------------------|
| 容器生命周期（建/启/停/删） | ✅ | ✅ | ✅ | ✅ | ✅ | 🧪 |
| 容器日志 / 持久终端 | ✅ | ✅ | 🟡 | ✅ | 🟡 | 🧪 |
| 镜像导入 / 拉取 | ✅ | ✅ | ✅ | ✅ | ✅ | 🧪 |
| 磁盘快照（建/树/回退/fork） | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 内存快照（CRIU dump） | ✅ | ❌ | 🧪 | 🧪 | ❌ | 🧪 |
| 同机内存回退（CRIU restore） | ✅ | ❌ | 🧪 | 🧪 | ❌ | 🧪 |
| 跨设备 `.mart` 迁移（文件层） | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| 跨设备迁移（进程内存） | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| GUI 流（mixcomp A） | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| GUI 流（B compositor） | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| 原生 Metal 窗口 | ✅ | ✅ | — | ❌ | ❌ | — |
| 输入 / 剪贴板 / IME | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| XWayland | ✅ | ✅ | ✅ | 🟡 | 🟡 | ✅ |
| Landlock 文件白名单 | 🧪 | ❌ | ✅ | 🧪 | ❌ | 🧪 |
| cgroup v2 freezer | 🟡 | 🟡 | ✅ | 🟡 | 🟡 | ✅ |
| FUSE 惰性挂载 | 🧪 | 🧪 | 🧪 | 🧪 | 🧪 | 🧪 |
| 自定义 guest 内核 | ✅ | ❌ | ❌ | ✅ | ❌ | ✅ |

> “GUI 流”是否可用与宿主无关，取决于容器里是否跑着 `mixum-agentd` + `mixcomp`（或 B compositor）；
> 宿主只负责 MAP/WSS 连接与窗口渲染。Windows 标 🟡 是因为本机未做完整窗口链路验证。

## 2. 逐宿主说明

### 2.1 macOS + Apple Container

- **运行时**：`container` CLI 1.5+，每个容器一个轻量 Linux VM。
- **内核**：默认 kata；可用 `container system kernel set --binary` 换成 `linux-mixum-micro`。
- **CRIU**：默认内核没有 `CHECKPOINT_RESTORE`，必须换自定义内核；CRIU 还需要容器内特权
  （2026-10-22 已在 Apple Container + `linux-mixum-micro` 上验证：Sway+foot 整个会话树
  dump → kill → restore，`swaymsg -t get_tree` 仍见 `app_id: foot`；mixcomp 窗口树需要
  whole-container dump，仍在接线。CRIU 版本用 3.17.1；4.2.1 在本内核 parasite SIGSEGV。）
  （`CAP_SYS_ADMIN`/`SYS_PTRACE`），Apple Container 是否放行待 PoC 验证。
- **内核是系统级**：换内核影响本机所有 Apple Container 容器，必须提供安装/回滚脚本。
- **磁盘快照**：可用；恢复后窗口通过 `guiSessions` 重放（Task 5）。
- **迁移**：`.mart` 文件层可跨设备；内存层不可。
- **原生 Metal**：MixumART 在 macOS 上用 MTKView 原生叠加层渲染 PNG 帧，✅。

### 2.2 macOS + Docker Desktop

- **内核固定**：Docker Desktop 的 LinuxKit VM 内核不可替换；CRIU 不提供。
- **磁盘快照 / `.mart` 迁移**：可用。
- **Landlock**：实测 Docker Desktop VM 内核未启用，任务层 Landlock 不生效（回退容器边界）。
- **Docker Desktop 只适合作为“能跑起来”的最低门槛**，不是完整能力宿主。

### 2.3 Linux + Docker Engine

- **内核 = 宿主内核**：CRIU、Landlock、cgroup、FUSE 都取决于宿主配置与容器特权。
- 宿主内核 ≥5.13 且开启 `CONFIG_SECURITY_LANDLOCK` 时，Task 4 的 Landlock 生效。
- CRIU 需要 privileged 容器 + `CHECKPOINT_RESTORE`；同内核约束仍然成立。
- 这是**能力最完整、最可控**的宿主，适合作为 CI 与 PoC 基准机。

### 2.4 Windows + WSL 2

- **内核可换**：`.wslconfig` 的 `[wsl2] kernel=` 指向 `linux-mixum-micro` 即可。
- 需额外满足 WSL2 的驱动要求（Hyper-V、9p、vsock）；`linux-mixum-micro` 的 config 已包含。
- CRIU 需要 WSL2 内 root；同内核约束成立。
- GUI：Tauri 在 Windows 上用 WebView canvas 渲染，未接原生纹理，标 🟡。

### 2.5 Windows + Docker Desktop

- 与 macOS Docker Desktop 同类：内核由 Docker Desktop 管理，不可替换；CRIU ❌。
- 其它文件层能力可用。

### 2.6 Linux + QEMU microVM

- 直接以 `linux-mixum-micro` 启动 microVM，能力最接近 Apple Container 的形态。
- MixumART 目前没有 QEMU runtime 后端，标 🧪；需要新增后端后才能产品化。
- 这是验证“无 hypervisor 厂商锁定”的参考路径。

## 3. 内核矩阵

| | `linux-mixum-micro` | `linux-mixum` |
|---|---|---|
| 基底 | kata 6.18.35 + mixum fragments | Arch 内核 config + mixum fragments |
| 目标 | Apple Container / WSL2 / QEMU microVM guest | 完整磁盘镜像 / 通用 Linux VM / 裸机 |
| 启动 | 亚秒级 | 2–5 s |
| 常驻 | 小（每实例省 30–80 MB） | 完整发行版内核 |
| CRIU | ✅ 编译开启（需特权） | ✅ 编译开启 |
| Landlock / FUSE | ✅ 编译开启 | ✅ 编译开启 |
| 硬件覆盖 | 窄（virtio/Hyper-V 为主） | 宽（发行版级驱动/模块/firmware） |
| 安装方式 | `container system kernel set` / `.wslconfig` | pacman 包 / 镜像 |
| 版本指纹 | `uname -r` 带 `-mixum-micro` | `uname -r` 带 `-mixum` |

> 两个内核共享同一份 config fragment 源；**内存快照要求节点内核指纹完全一致**，
> 升级内核会让旧 memory/hybrid 节点失效（disk 节点已放开跨内核）。

## 4. 跨设备迁移矩阵

迁移单元 = **镜像 digest + 容器规格 + `.mart` 快照 + 节点元数据（含 guiSessions）**。
不是“把活容器搬过去”。

| 源 → 目标 | 磁盘快照 | 内存快照 | GUI 窗口 | 说明 |
|---|---|---|---|---|
| 同宿主同 runtime | ✅ | 🧪 | ✅ 重放 | CRIU 仅同内核 |
| WSL2 → Apple Container（同 arm64） | ✅ | ❌ | ✅ 重放 | 同架构、同镜像 |
| WSL2 → Linux Docker（同架构） | ✅ | ❌ | ✅ 重放 | 目标镜像需可拉取 |
| Apple Container → Linux Docker（同 arm64） | ✅ | ❌ | ✅ 重放 | 同上 |
| x86_64 → arm64 | ❌ | ❌ | — | 二进制不兼容；只能迁源码后重建 |
| Docker Desktop → 任意 | ✅ | ❌ | ✅ 重放 | 源侧无 CRIU |
| 任意 → QEMU microVM | ✅ | ❌ | ✅ 重放 | 需 QEMU 后端 |

**跨设备不迁移的东西**：运行中的进程、终端滚动缓冲、容器内 `pacman -S` 的包、`/etc` 改动、
令牌/证书、端口映射、未落盘的编辑器缓冲。

## 5. 物理/产品边界（不做）

1. **跨 hypervisor 的 CRIU**：WSL2（Hyper-V）↔ Apple Container（virtio）↔ Docker Desktop
   （另一套 virtio 拓扑）之间，即使内核版本相同，设备 fd 与 virtio 状态也无法对上。
2. **跨架构内存迁移**：x86_64 与 arm64 之间不存在通用的进程内存迁移。
3. **Docker Desktop 自定义内核**：内核由 Docker Desktop 管理，用户态无法替换。
4. **Wayland 会话原地恢复**：没有标准状态序列化；跨设备只能重启应用 + 窗口重放。
5. **无特权 CRIU**：CRIU 需要 `CAP_SYS_ADMIN`/`SYS_PTRACE`；不提供特权变体就不做内存快照。

## 6. 宿主验收清单

在任何宿主上宣称“支持”前，至少跑完：

```sh
# 1. 运行时
<runtime> version && <runtime> ps

# 2. 容器内 agentd / mixcomp
mixumd --config /dev/null &            # 看 READY
mixcomp --sock /run/mixum/display.sock &

# 3. 磁盘快照
#    GUI: 创建 → 改文件 → restore → 文件回滚；窗口按 guiSessions 重开

# 4. 内存快照（仅 🧪 宿主）
criu check && criu dump --tree <pid> --images-dir /tmp/d --shell-job
criu restore --images-dir /tmp/d --shell-job

# 5. 迁移
#    导出 .mart → 目标导入 → 起同镜像容器 → restore → 窗口重放
```

## 7. 与其它文档的关系

- 运行时契约：[`runtime-contract.zh-CN.md`](runtime-contract.zh-CN.md)
- 显示架构：[`display.md`](display.md)
- 内核构建与验收：`mixumos/kernel/README.md`
- 迁移包格式：`mixumos/docs/migration.md`（待补）
