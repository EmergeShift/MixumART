# 模块地图：每块代码该放在哪

这份文档回答一个问题：**我要改的东西，应该落在哪个目录？**

改动前先在这里找到对应的行；找不到时，先在 `docs/` 补一行，再写代码。

## 1. 目录结构

```
MixumART/
├── Cargo.toml                      # Rust workspace：成员、共享依赖版本
├── crates/
│   ├── mixum-env-core/             # 领域类型 + 后端契约（唯一的跨组共享 Rust 代码）
│   │   └── src/{container,error,process,runtime}.rs
│   ├── mixum-env-runtime-docker/   # Docker 后端
│   ├── mixum-env-runtime-apple/    # Apple Container 后端（macOS）
│   └── mixum-env-runtime-wsl2/     # WSL 2 后端（Windows，并行组）
├── apps/
│   └── desktop/
│       ├── src-tauri/              # Tauri 壳：命令、状态、窗口、权限
│       │   ├── src/lib.rs          # 装配后端 + 注册命令 + PATH 修补
│       │   ├── src/commands.rs     # 所有 #[tauri::command]
│       │   ├── capabilities/       # Tauri v2 能力（默认只开 core:default）
│       │   └── tauri.conf.json
│       ├── src/                    # React 界面
│       │   ├── App.tsx             # 侧栏 + 顶栏 + 视图切换
│       │   ├── api.ts              # 唯一的 invoke 出口
│       │   ├── types.ts            # core 类型的 TS 镜像
│       │   ├── status.ts           # 状态文案/色调映射
│       │   ├── views/              # 页面级组件
│       │   └── components/         # 可复用小组件
│       └── package.json
├── docs/
└── README.md
```

## 2. 依赖方向（只允许单向）

```
apps/desktop/src  ──invoke──▶  apps/desktop/src-tauri
                                      │
                                      ▼
                            crates/mixum-env-core
                                      ▲
                    ┌─────────────────┼─────────────────┐
        runtime-docker          runtime-apple        runtime-wsl2
```

硬规则：

- `mixum-env-core` **不依赖**任何 runtime crate，也不依赖 Tauri；
- runtime crate 之间**不互相依赖**；
- Tauri 层不直接 `Command::new`，所有外部调用经后端 crate；
- 前端不直接写命令名字符串，统一走 `src/api.ts`。

## 3. 模块归属表

| 路径 | 负责什么 | 谁可以改 | 不该出现 |
|------|----------|----------|----------|
| `crates/mixum-env-core` | 领域类型（`Container`/`CreateSpec`/`PortMapping`）、`RuntimeBackend` 契约、`RuntimeStatus`、`EnvError`、带超时的命令调用 | 全体；**契约变更需两组 review** | CLI 参数、`cfg(target_os)` 分支、Tauri 类型、界面文案 |
| `crates/mixum-env-runtime-docker` | `docker` CLI 探测、列举、输出解析；（M1）写操作 | 本组 | React/Tauri 依赖、其它 runtime 的代码 |
| `crates/mixum-env-runtime-apple` | `container` CLI 探测、列举、输出解析；（M2）写操作 | 本组 | 同上 |
| `crates/mixum-env-runtime-wsl2` | `wsl.exe` 交互、发行版管理 | **WSL 2 组** | 同上；不要改 core 去迁就实现 |
| `apps/desktop/src-tauri/src/commands.rs` | 命令：取状态、转错误码、返回给前端 | 本组 | 业务逻辑、平台细节 |
| `crates/mixum-env-surface` | surface/frame/input wire 类型与编解码 | 显示组 | 平台/CLI/Tauri 类型 |
| `crates/mixum-env-compositor` | 容器内 Wayland 合成器、buffer/damage/seat | 显示组 | 宿主 UI、Tauri 依赖 |
| `crates/mixum-env-display` | 宿主 TCP 调试客户端、窗口合成、输入回传 | 显示组 | Tauri 私有状态 |
| `crates/mixum-env-map` | MAP WSS/TOFU + display.list/attach/input | 显示组 | 平台/CLI 细节 |
| `apps/desktop/src-tauri/src/lib.rs` | 后端装配、命令注册、启动期环境修补 | 本组 | 业务逻辑 |
| `apps/desktop/src/views/` | 页面（环境、容器、后续镜像/设置） | 前端 | 命令名、后端私有字段 |
| `apps/desktop/src/components/` | 无状态小组件（徽章等） | 前端 | 数据请求 |
| `apps/desktop/src/{api,types,status}.ts` | 契约镜像与展示映射 | 前端 + core 同步 | 业务分支 |
| `docs/` | 归属、契约、构建说明 | 全体 | — |

> 「谁可以改」不是权限，是**冲突成本**：改别人负责的目录请在 PR 里说明原因，
> 并在合并前找对应负责人过一眼。

## 4. 常见改动落点

| 我要做的事 | 改哪里 |
|------------|--------|
| 新增一个 runtime（如 Podman） | `core/runtime.rs` 加 `RuntimeKind` 变体 → 新建 `crates/mixum-env-runtime-podman` → `lib.rs` 的 `build_registry` 加分支 → `docs/` 补一行 |
| 给容器列表加字段（如 CPU/内存） | `core/container.rs` 的 `Container` → 各后端解析各自实现 → `src/types.ts` → 表格列 |
| 新增容器操作（启动/停止/删除） | 先在 `docs/runtime-contract.md` 定语义 → 后端 crate 实现 trait 方法 → `commands.rs` → `api.ts` → UI |
| 新增 Tauri 命令 | `commands.rs` → `lib.rs` 的 `generate_handler!` → `api.ts` 包一层 |
| 新增页面 | `src/views/` → `App.tsx` 的 Tab → `docs/ui.md` 补视图表 |
| 改状态文案 | 只展示 → `src/status.ts`；改协议字段 → `core/runtime.rs`（走契约流程） |
| 新增错误种类 | `core/error.rs` + `docs/runtime-contract.md` 的错误映射表 |
| 处理平台差异 | 对应 runtime crate 内部，用 `cfg`；**不要**放进 core |
| 升级依赖 | Rust：根 `Cargo.toml` 的 `[workspace.dependencies]`；前端：`apps/desktop/package.json` |
| 加构建/发布脚本 | 新建 `scripts/`（目前没有） |

## 5. 并行协作规则（WSL 2 组）

- WSL 2 组只改 `crates/mixum-env-runtime-wsl2/**`，以及自己新增的文档；
- 该 crate 已经注册进 GUI，在 Windows 上会显示「接入中」，所以**不需要**本组配合就能联调；
- 如果契约不够用（缺字段、缺错误、缺 trait 方法）：
  1. 先改 `docs/runtime-contract.md` 说明动机；
  2. PR 标题带 `[contract]`；
  3. 本组与 WSL 2 组各一人 review 后再合；
- 共享文件（根 `Cargo.toml`、`lib.rs`、`types.ts`）的改动保持小步，冲突时由本组统一合并；
- 分支名建议：`feat/wsl2-import`、`fix/docker-parse`；一个 PR 只覆盖自己模块。

## 6. 契约变更清单

改 `mixum-env-core` 的公开 API 时，逐项确认：

- [ ] `docs/runtime-contract.md` 已更新
- [ ] 三个后端都能编译（Windows 也不能落下）
- [ ] `src/types.ts` 已同步
- [ ] 序列化有单测覆盖（字段名是契约的一部分）
- [ ] 旧后端暂不支持时，用 `Option` + `serde(default)`，不要直接删字段
