# mixum Agent Runtime

宿主机上的 mixumOS 入口。一个 Tauri 2 桌面应用：

- 把 mixumOS 接到本机已有的容器运行时上：**WSL 2 / Docker / Apple Container**；
- 作为 mixumOS 容器的管理器（列表，后续接管创建与启停）；
- 未来承接原本在 mixumOS 内的交互设计（任务、窗口、快照入口）。

## 当前状态

**运行时管理**：三种运行时已注册；Docker、Apple Container、WSL 2 均支持
探测、列举、创建/导入、启停和删除。环境页还支持连接档案、监看、快照、
命令终端和日志。

**显示原型（B0/B1/B2）**：`crates/mixum-env-compositor` 在容器内跑 Smithay
headless 合成器，把每个 surface 的 buffer+damage 经 TCP/MAP 发给宿主。
macOS 上 Tauri surface 窗口默认使用 **MTKView + Metal** 原生叠加层渲染
PNG 帧（非 macOS 或 `MIXUM_NATIVE_METAL=0` 时回退 WebView canvas）。
输入、IME、剪贴板仍由 WebView 处理。

Tauri 的「窗口」页通过 MAP WSS（`mixum-env-map`）连接 agentd：
输入地址/令牌/可选指纹，扫描 surface、为每个 surface 开独立窗口。
令牌复用 A 的 MAP 令牌；`display.takeover` 已接入窗口页，支持按钮或
Ctrl/Cmd+Shift+T。agentd 已支持多个 comp 源，mixcomp（A）与 B compositor
可以同时显示，窗口页以 A/B 标签区分并支持 A 回退筛选。host→remote 粘贴走键/文本事件，remote→host
剪贴板用 `Cmd/Ctrl+Alt+V` 拉取。
`mixum-env-display` 的 minifb 查看器保留作直连 TCP 调试。

已接：Metal 原生纹理、subsurface/popup 合成与输入、host paste、
text-input IME（text-input 客户端；foot 终端本身不消费 commit）、
remote→host 剪贴板、**XWayland**（X11 toplevel、override-redirect 菜单、
指针/键盘、X11↔Wayland 剪贴板；需要镜像内 `xorg-xwayland`）。
X11 文本：`XMODIFIERS=@im=mixum` 走内置 XIM server；无 XIM 时回退
XTEST/keymap 直接注入。agentd 任务和登录 shell 会从
`/run/mixum/xwayland.display` 自动注入 `DISPLAY`/`XMODIFIERS`，应用无需
自己带参数。host 文本/URI/文本文件内容可拖入 surface；真正的
跨 VM XDND 文件 drag-out/drop-target 与 GPU/dmabuf 零拷贝仍未接。详见
[`docs/display.md`](docs/display.md)。

| 运行时 | 探测 | 列容器 | 创建 / 启停 |
|--------|------|--------|-------------|
| Docker | ✅ | ✅ | ✅ 创建 / 启停 / 删除 |
| Apple Container | ✅ | ✅ | ✅ 创建 / 启停 / 删除 |
| WSL 2 | ✅ | ✅ | ✅ 导入 / 启停 / 注销 |

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
| WSL 2 | — | 代码已实现；本机验证探测 / 列举 / 幂等，导入未跑真实 rootfs | — |

## 文档

| 文档 | 内容 |
|------|------|
| [architecture.md](docs/architecture.md) | 定位、与 mixumOS 的分工、进程模型、路线图 |
| [module-map.md](docs/module-map.md) | **每块代码该放在哪**、模块归属、并行协作规则 |
| [runtime-contract.md](docs/runtime-contract.md) | 后端契约、状态语义、错误映射、序列化约定 |
| [display.md](docs/display.md) | B 路线：容器内合成器、surface/damage/input、B0/B1 运行方式 |
| [ui.md](docs/ui.md) | 界面结构、视图表、设计 token、文案约定 |
| [contributing.md](docs/contributing.md) | 环境、命令、跨平台规则、提交与已知坑 |

## 许可证

[Apache-2.0](LICENSE)
