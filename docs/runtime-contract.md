# 运行时后端契约

所有 runtime 后端共享同一份契约。**它定义了并行开发的边界**：WSL 2 组实现自己的
`RuntimeBackend`，本组不需要改 GUI。

代码位置：`crates/mixum-env-core/src/runtime.rs`。

## 1. 契约

```rust
#[async_trait]
pub trait RuntimeBackend: Send + Sync {
    fn kind(&self) -> RuntimeKind;
    async fn probe(&self) -> RuntimeStatus;
    async fn list_containers(&self) -> Result<Vec<Container>, EnvError>;

    // 以下四个在 M0 有默认实现：直接返回 EnvError::Unimplemented。
    async fn create_container(&self, spec: CreateSpec) -> Result<Container, EnvError>;
    async fn start_container(&self, id: &str) -> Result<(), EnvError>;
    async fn stop_container(&self, id: &str) -> Result<(), EnvError>;
    async fn remove_container(&self, id: &str) -> Result<(), EnvError>;
}
```

写操作在 M0 有默认实现，返回 `EnvError::Unimplemented`。**后端不实现也要能编译**，
这样 UI 可以先长出来。

## 2. `probe` 的语义

`probe` 是状态查询，不是错误通道：

| 返回 | 含义 | 典型触发 |
|------|------|----------|
| `Ready { version, detail }` | CLI 存在且 daemon 正常 | `docker version` 成功；`container system status` 报 `running` |
| `NotInstalled { hint }` | 可执行文件不在 PATH | `ENOENT` |
| `NotRunning { hint }` | 工具在，服务没起来 | Docker Desktop 未启动、`container system` 未 start |
| `Unsupported { reason }` | 平台不支持，或后端尚未接入 | macOS 上的 WSL 2；占位实现 |

硬性要求：

- **不返回 `Err`**：daemon 挂了是状态，不是异常；
- **不 panic**、不阻塞超过 5 秒；
- 无副作用，不修改任何配置；
- `hint` 是给用户看的下一步操作（例如「执行 container system start」），不是日志。

## 3. 方法语义

| 方法 | 成功 | 失败 |
|------|------|------|
| `list_containers` | 返回**含已停止**的容器；没有容器返回空 `Vec` | CLI 缺失 → `NotInstalled`；输出无法解析 → `Parse`；非零退出 → `CommandFailed` |
| `create_container` | 返回创建后的摘要（状态可以是 `created`） | 重名、镜像不存在等映射为 `CommandFailed`（M1 细化错误类型） |
| `start_container` | 容器进入 `running` | 幂等约定：**已运行视为成功** |
| `stop_container` | 容器进入 `stopped` | 幂等约定：**已停止视为成功** |
| `remove_container` | 容器消失 | 幂等约定：**不存在视为成功** |

幂等约定的目的：UI 重试与并发点击不应产生用户无法理解的错误。

WSL 2 的字段映射、写操作超时与额外的幂等细节见 §9。

## 4. 外部命令规则

统一走 `mixum_env_core::process::run`：

```rust
process::run("docker", &["ps", "-a", "--no-trunc", "--format", "{{json .}}"], LIST_TIMEOUT).await
```

- 只传 argv，**不经过 shell**；命令里不允许出现用户输入拼接的字符串；
- 超时：探测 5s，列举 10s；WSL 2 写操作 create 10min（rootfs tar 可达 GB 级）、
  start/stop/remove 30s；超时返回 `EnvError::Timeout`；
- `stdin` 一律关闭，避免子进程等待输入；
- 可执行文件找不到时统一返回 `NotInstalled`，安装提示由后端补充；
- **编码**：`process::run` 按 UTF-8 解码；wsl.exe 等控制台工具输出
  UTF-16LE，走 `process::run_raw`（返回原始字节）由后端自行解码
  （启发式见 wsl2 后端 `decode_console`）。

## 5. 错误映射

`EnvError` → 命令层字符串 → 前端错误条。展示文案由 `Display` 决定。

| 错误 | 触发 | 用户看到 |
|------|------|----------|
| `Unsupported` | 平台不符 / 未接入 | 灰色徽章 + 原因（`probe`） |
| `NotInstalled` | `ENOENT` | 红色徽章 + 安装命令（`probe`） |
| `Timeout` | 超过超时 | 错误条：命令 + 秒数 |
| `CommandFailed` | 非零退出 | 错误条：命令 + stderr 首行 |
| `Parse` | 输出格式变化 | 错误条：提示可能升级了 runtime |
| `Io` | 进程无法启动 | 错误条 |
| `Unimplemented` | 骨架占位 | 错误条（M1 前不应从 UI 触达） |

`probe` 不返回错误，服务未启动体现为 `RuntimeStatus::NotRunning`（橙色徽章 + 启动提示）。

## 6. 序列化契约（前后端共享）

- `RuntimeKind` 用 **snake_case** 字符串：`docker` / `wsl2` / `apple_container`；
- 结构体字段用 **camelCase**（`createdAt`、`readOnly`）；
- `RuntimeStatus` 用 `state` 做判别字段：

```json
{
  "kind": "apple_container",
  "name": "Apple Container",
  "status": { "state": "ready", "version": "1.5.0", "detail": null }
}
```

```json
{
  "id": "map-node",
  "name": "map-node",
  "image": "127.0.0.1:15000/mixumos:fix1",
  "state": "running",
  "status": null,
  "createdAt": "2026-10-18T13:02:44Z",
  "ports": [{ "host": 17601, "container": 7601, "protocol": "tcp" }]
}
```

前端镜像在 `apps/desktop/src/types.ts`；**字段名是契约的一部分**，改名前先改这份文档。

## 7. 实现一个新后端

1. 新建 `crates/mixum-env-runtime-<name>/`，依赖 `mixum-env-core`；
2. 实现 `RuntimeBackend`，在 `crates/mixum-env-runtime-<name>/src/lib.rs` 暴露
   `pub fn backend() -> Arc<dyn RuntimeBackend>`；
3. 在 `mixum-env-core/src/runtime.rs` 的 `RuntimeKind` 加变体（`ALL` 与 `id()`）；
4. 在 `apps/desktop/src-tauri/src/lib.rs` 的 `build_registry()` 加分支；
5. 在 `docs/module-map.md` 的归属表补一行。

平台专属的后端（WSL 2）也要在其它平台编译通过：`probe` 返回 `Unsupported` 即可。

## 8. 测试要求

- **纯函数解析必须有单测**，样例直接取真实 CLI 输出（见两个后端的 `mod tests`）；
- 序列化字段名用单测锁定；
- 依赖本机 runtime 的端到端检查用 `#[ignore]` 标记，不阻塞 CI：

```sh
cargo test --workspace
cargo test -p mixum-env-runtime-apple -- --ignored   # 需要本机 container 服务
cargo test -p mixum-env-runtime-docker -- --ignored  # 需要本机 docker daemon
cargo test -p mixum-env-runtime-wsl2 -- --ignored    # 需要本机 WSL
```

## 9. WSL 2 写操作（并行组，2026-10-19）

WSL 2 没有「容器」这一层：一个 mixumOS 节点 = 一个 `mixum-*` 发行版，由
`wsl --import` 从 rootfs tar 导入。所以写操作的字段映射和 Docker 不同，
**语义以本节为准**。

| 方法 | 等价 CLI | 超时 |
|------|----------|------|
| `create_container` | `wsl --import <name> <installDir> <rootfs.tar>` | 10 分钟 |
| `start_container` | `wsl -d <id> -e /bin/true` | 30s |
| `stop_container` | `wsl --terminate <id>` | 30s |
| `remove_container` | `wsl --unregister <id>` | 30s |

`CreateSpec` 在 WSL 2 下的字段映射：

| 字段 | 含义 |
|------|------|
| `name` | 发行版名，**必须** `mixum-` 前缀（与列表过滤一致，不区分大小写） |
| `image` | **本地 rootfs tar 路径**——WSL 没有镜像仓库，节点来自 mixumOS `dist/` 产物 |
| `ports` / `mounts` / `env` / `command` | 无对应物，**非空即报错**，不静默忽略 |

安装位置：`%MIXUM_WSL_INSTALL_ROOT%\<name>`；未设置时
`%LOCALAPPDATA%\mixum\wsl\<name>`，目录不存在则创建。

硬性要求：

- **只动 `mixum-` 前缀的发行版**：`start` / `stop` / `remove` 收到其它名字直接拒绝，
  避免把宿主自己的 `Ubuntu-24.04` / `docker-desktop` terminate 或 unregister 掉；
- `remove` 会删掉 `ext4.vhdx`，**不可逆**，UI 层必须二次确认；
- 幂等：`stop` / `remove` 遇到不存在的发行版视为成功；`create` 重名视为失败；
  `start` 遇到不存在的发行版是错误（调用方本来就知道自己要点哪个）；
- `start` 只把发行版实例拉起。**WSL 在无进程约 15s 后自动回到 `Stopped`**，
  `Running` 仅表示「当前有活动会话」；mixumOS 常驻服务（`mixumd` / entrypoint）
  的启动语义在后续里程碑以新契约补充；
- 是否「存在」以 `wsl --list --verbose` 的解析结果为准，不去猜错误文案；
- 「一个发行版都没有」不是错误：`list_containers` 返回空 `Vec`。
