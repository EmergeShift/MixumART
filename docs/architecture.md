# 架构

## 1. 这个项目是什么

mixum Agent Env 是**宿主机上安装的那一层**：一个 Tauri 桌面应用，负责

1. 把 mixumOS 接到宿主已有的容器运行时上（WSL 2 / Docker / Apple Container）；
2. 作为 mixumOS 容器的管理器（列表、生命周期、后续的镜像与内核准备）；
3. 承接原本长在 mixumOS 内部、现在应该放到宿主机上的交互设计。

mixumOS（`../mixumos`）是底座：容器里的 `mixumd`、合成器 mixcomp、快照引擎、
`wss://host:7601` 协议。本仓库不重新实现它们，只消费。

## 2. 与 mixumOS 的分工

| 关注点 | mixumOS（底座） | Agent Env（本仓库） |
|--------|----------------|---------------------|
| 容器内守护进程 | `mixum-agentd` / `mixumd` | 不涉及 |
| 协议 | 协议 v1（WSS + envelope） | M4 起作为客户端接入 |
| 快照 / 任务引擎 | Rust crates | M4 起提供入口 |
| 宿主运行时 | 不负责（v1 非目标） | **本仓库的核心职责** |
| 安装与首次启动 | 镜像 / 发行版构建 | 宿主机安装器（M4） |

> `mixumos/docs/architecture.md` 的 v1 非目标里写着「Windows/macOS 作为容器宿主
> （只作为客户端）」。本仓库正是把这条限制打开后的产物：**两边文档各自维护**，
> 宿主形态以本仓库为准；mixumOS 侧文档待其维护者同步。

## 3. 进程模型与数据流

单进程 + 一个 webview：

```
React (webview)
   │  invoke("list_containers", { kind })
   ▼
src-tauri/commands.rs
   │
   ▼
BackendRegistry ──▶ Arc<dyn RuntimeBackend>
                        │
                        ▼
              process::run(cli, args, timeout)   # argv 直传，不过 shell
                        │
                        ▼
              docker ps / container list / wsl.exe
                        │
                        ▼
              parse → Vec<Container> (serde) ──▶ JSON ──▶ UI
```

- **后端注册**：`lib.rs` 的 `build_registry()` 按 `RuntimeKind::ALL` 装配，
  新增 runtime 只加一个分支。
- **命令层**：`commands.rs` 只做参数透传与错误转字符串，平台逻辑零行。
- **并发**：命令是 async，Tauri 自带 tokio 运行时；外部调用统一带超时
  （探测 5s、列举 10s），daemon 卡住不会冻结 UI。
- **状态**：启动时探测一次，之后用户手动刷新；没有后台轮询。

## 4. 平台支持

| 能力 | macOS | Windows | Linux |
|------|-------|---------|-------|
| Apple Container | ✅ 已验证（只读） | — | — |
| Docker | ✅ 已验证（只读） | 代码就绪，未真机验证 | 代码就绪，未真机验证 |
| WSL 2 | — | 由并行组接入中 | — |
| 构建依赖 | Xcode CLT | WebView2 + MSVC | webkit2gtk 等（见 Tauri 文档） |

「已接入（只读）」= 探测与列举可用，创建/启停/删除返回 `Unimplemented`，
界面暂不提供按钮（见路线图 M1/M2）。

## 5. 安全与边界

- **不提权**：所有外部命令以当前用户执行；需要管理员权限的能力（例如启用
  WSL 功能）在 M3 用系统授权流程，不在应用内自行提权。
- **不过 shell**：`process::run` 只接受 argv，拒绝拼接命令字符串。
- **超时**：任何 CLI 调用都有上限，避免 daemon 异常时挂死。
- **不联网**：M0 没有网络请求；M4 连接 mixumd 时只连回环地址或用户显式配置的
  节点，令牌入系统钥匙串，不写日志。
- **权限最小化**：Tauri capability 只开 `core:default`，插件权限按需追加。
- **无遥测**。

## 6. 路线图

| 里程碑 | 内容 | 完成标准 |
|--------|------|----------|
| **M0（当前）** | Tauri 2 骨架、三后端注册、只读探测与列举、模块归属文档 | macOS 编译/运行验证通过；Windows/Linux 待 CI；macOS 上能列出 Docker 与 Apple Container 容器 |
| M1 | Docker 写操作（创建/启停/删除）、容器详情与日志 | 通过 GUI 完整跑起一个 mixumOS 容器 |
| M2 | Apple Container 写操作、OCI 镜像导入、内核设置 | 一键起节点，等价于 apple-container.md 的手工流程 |
| M3 | WSL 2 接入（并行组） | Windows 上导入发行版并起节点 |
| M4 | 宿主机安装器、mixumOS 设计迁移（任务/窗口/快照入口） | 新机器一条链路装完并能管理节点 |

## 7. v0 非目标

- 多用户 / 多租户；
- 远程节点编排（云侧已有 operator）；
- 镜像构建（由 mixumOS 的 `dist/` 负责）；
- 裸机（非容器）部署。
