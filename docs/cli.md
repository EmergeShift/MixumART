# mixum CLI

`mixum` 是 MixumART 的本地控制面 CLI。它直接通过 Docker / Apple Container /
WSL 2 后端管理 MixumOS 环境，不依赖 Tauri 窗口，也不使用 `mart` 这个命令名。

```text
mixum          用户/Agent CLI
mixum-artd     后续常驻宿主控制面（本版本尚未暴露命令）
mixumd         容器内 agentd，保持原名
mixcomp        容器内 compositor，保持原名
```

## 稳定命令面

```text
mixum doctor
mixum runtime list
mixum env list [--all]
mixum env create --profile mixumos --title TITLE [--method subagent] [--image IMAGE]
mixum env show SELECTOR
mixum env start SELECTOR
mixum env stop SELECTOR
mixum env remove SELECTOR [--force]
mixum env windows SELECTOR [--timeout 30]
mixum env logs SELECTOR [--tail 200]
mixum env terminal SELECTOR [--shell /bin/sh]
mixum env snapshot list SELECTOR
mixum env snapshot create SELECTOR [--label LABEL]
mixum env snapshot restore SELECTOR NODE --yes
mixum port check PORT
mixum port list
mixum desktop
mixum mcp
mixum version
```

所有命令支持全局 `--json`。没有实现的功能不会出现在命令表里，也不会返回
“TODO/Unsupported” 占位。

## 数据目录

优先级：

1. `--data-dir`
2. `MIXUM_DATA_DIR`
3. `$XDG_DATA_HOME` / `~/Library/Application Support` 下的
   `dev.mixum.agentenv/mixum-cli`

文件：

```text
state.json      环境标题、method、runtime、端口、指纹；权限 0600
state.lock      create 的互斥锁
```

`state.json` 损坏时 CLI 直接报错，不会静默重建；这是为了避免丢掉环境归属信息。

## 环境列表

`mixum env list` 默认只显示已托管或镜像名包含 `mixumos` 的环境，避免把构建/普通
容器混进 Agent 环境列表。`--all` 才会列出 runtime 里的全部容器。

## Runtime 选择

`doctor` / `runtime list` 会显示三个后端。

- `mixum env create` 未指定 `--runtime` 时：
  - macOS 优先 Apple Container；
  - Windows 优先 WSL 2；
  - 其他平台优先 Docker；
  - 如果首选不可用，使用下一个 ready 的后端。
- 指定 `--runtime` 时，该后端必须 ready，否则返回错误。
- 多个后端同时 ready 时，创建输出会明确记录实际后端。

## Profile 与镜像

### `--profile mixumos`

要求镜像引用包含 `mixumos`，并且镜像本地存在。镜像必须带：

- `/entrypoint.sh`
- `mixumd`（agentd）
- `mixcomp`
- `7601/tcp`

省略 `--image` 时：

- 只有一个本地 mixumos 镜像 → 使用它；
- 多个 → 报错并列出候选，要求显式 `--image`；
- 没有 → 报错，提示先 pull/load。

### `--profile generic`

必须显式 `--image`。不保证 MAP 或窗口，只创建容器；默认不等待 MAP。
适合构建/工具容器。

## 端口

- 容器内 MAP 端口固定为 `7601/tcp`。
- 宿主机只发布到 `127.0.0.1`，不暴露局域网。
- 默认从 `17600-17999` 选第一个可绑定且未被 mixum 环境占用的端口。
- `--port N` 使用指定端口；端口被占用、被其他环境发布时返回冲突。
- `--port 0` 等价于自动分配。
- 创建期间持有 `state.lock`，避免两个并发 create 选到同一端口。
- `mixum port check N` 只检查本机 `127.0.0.1` 绑定；`port list` 列出 runtime 已发布端口。

## Selector

`SELECTOR` 可以是：

- 完整容器 id
- 容器名
- `state.json` 中的 title
- id/name 的前缀（至少 4 个字符）

歧义时返回错误并列出候选；不会猜测。

## 错误与退出码

| code | 含义 |
| --- | --- |
| 0 | 成功 |
| 1 | IO / 状态文件错误 |
| 2 | 参数错误 |
| 3 | 找不到环境/镜像 |
| 4 | 冲突（端口、重名、运行中删除、歧义） |
| 5 | runtime 不可用或命令失败 |
| 6 | MAP/agentd 失败 |

`--json` 模式失败时 stdout 输出：

```json
{"error":{"code":4,"message":"..."}}
```

成功输出是命令特定的 JSON；token 永远不会出现在输出中。

## JSON 示例

### create

```json
{
  "environment": {
    "id": "mixum-cli-probe-01m5h9",
    "name": "mixum-cli-probe-01m5h9",
    "title": "CLI probe",
    "runtime": "apple_container",
    "image": "127.0.0.1:15000/mixumos:0.1.4",
    "state": "running",
    "method": "subagent",
    "managed": true,
    "mapPort": 17777,
    "mapAddr": "127.0.0.1:17777",
    "fingerprint": "50:d2:..."
  },
  "mapReady": true,
  "fingerprint": "50:d2:...",
  "windows": 0
}
```

### windows

```json
{
  "environment": "mixum-cli-probe-01m5h9",
  "fingerprint": "50:d2:...",
  "windows": [
    {"id": 1, "title": "foot", "appId": "foot", "width": 300, "height": 169}
  ]
}
```

## 安全

- admin-token 通过本地 runtime `exec` 从容器读取，按需使用，不写入 CLI state；
- CLI 不输出 token；`env logs` 会把 `map1.…` 脱敏；
- MAP 使用 TOFU + 指纹固定；首次指纹写入 `mixum-env-map` 的 known-hosts；
- 终端使用宿主 `exec -it`，是本地特权路径；CLI 只允许交互式 TTY；
- 这仍是 admin-token 方案，证书/公钥认证尚未实现；不要把 token 写进镜像或快照。

## MCP

`mixum mcp` 启动 stdio MCP server，使用同一份 CLI 实现，工具不会返回 token。

```text
mixum_env_list
mixum_env_create
mixum_env_show
mixum_env_windows
mixum_env_start
mixum_env_stop
mixum_snapshot_list
mixum_snapshot_create
mixum_port_check
```

终端、任意 shell 和 snapshot restore 不通过 MCP 暴露，避免把本地特权路径
误当成只读监看能力。配套 skill 在 `skills/mixum-art/SKILL.md`。

## Daemon

宿主侧 Unix socket API 是 `mixum-artd`，与 CLI 同 crate：

```bash
cargo build -p mixum-cli --release
./target/release/mixum-artd
```

它用 sibling `mixum --json` 执行同一套规则，socket 权限 0600；协议见
[docs/daemon.md](daemon.md)。

## 构建

```bash
cargo build -p mixum-cli --release
./target/release/mixum --help
```

当前版本提供 `mixum mcp`（stdio）和 `mixum desktop`。`mixum serve` 浏览器 UI
尚未实现，不会先放空壳命令。
