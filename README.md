# mixum Agent Env

宿主机上的 mixumOS 入口。一个 Tauri 2 桌面应用：

- 把 mixumOS 接到本机已有的容器运行时上：**WSL 2 / Docker / Apple Container**；
- 作为 mixumOS 容器的管理器（列表，后续接管创建与启停）；
- 未来承接原本在 mixumOS 内的交互设计（任务、窗口、快照入口）。

## 当前状态

**M0 骨架**：三种运行时已注册，Docker 与 Apple Container 可探测、可列出容器
（含已停止）；写操作只留了契约。WSL 2 后端由并行小组接入中。

| 运行时 | 探测 | 列容器 | 创建 / 启停 |
|--------|------|--------|-------------|
| Docker | ✅ | ✅ | 契约预留（M1） |
| Apple Container | ✅ | ✅ | 契约预留（M2） |
| WSL 2 | 占位 | 占位 | 并行组（M3） |

## 快速开始

```sh
cd apps/desktop
npm install
npm run tauri dev
```

构建安装包：

```sh
cd apps/desktop && npm run tauri build
```

## 目录

```
crates/
  mixum-env-core/             # 领域类型 + 运行时契约 + 带超时的命令调用
  mixum-env-runtime-docker/   # Docker 后端
  mixum-env-runtime-apple/    # Apple Container 后端（macOS）
  mixum-env-runtime-wsl2/     # WSL 2 后端（Windows，并行组）
apps/desktop/
  src-tauri/                  # Tauri 壳：命令、状态、窗口、能力
  src/                        # React 界面
docs/                         # 架构、模块归属、契约、界面、协作
```

## 平台

| 能力 | macOS | Windows | Linux |
|------|-------|---------|-------|
| Apple Container | ✅ 已验证 | — | — |
| Docker | ✅ 已验证 | 代码已实现，未在真机验证 | 代码已实现，未在真机验证 |
| WSL 2 | — | 接入中 | — |

## 文档

| 文档 | 内容 |
|------|------|
| [architecture.md](docs/architecture.md) | 定位、与 mixumOS 的分工、进程模型、路线图 |
| [module-map.md](docs/module-map.md) | **每块代码该放在哪**、模块归属、并行协作规则 |
| [runtime-contract.md](docs/runtime-contract.md) | 后端契约、状态语义、错误映射、序列化约定 |
| [ui.md](docs/ui.md) | 界面结构、视图表、设计 token、文案约定 |
| [contributing.md](docs/contributing.md) | 环境、命令、跨平台规则、提交与已知坑 |

## 许可证

[Apache-2.0](LICENSE)
