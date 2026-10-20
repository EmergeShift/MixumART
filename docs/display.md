# 显示与 Wayland 合成器（B 路线，设计/实现）

> 状态：**B0/B1/B2 已实现**（2026-10-20）。容器内 Smithay 合成器把
> toplevel + subsurface/popup 的 `wl_shm` buffer + damage 转发到宿主；
> 指针/键盘经 MAP 回传到 seat，text-input IME 与剪贴板已接。A 路线（mixcomp 捕获 + damage 补丁 + PNG）
> 在 mixumOS 侧独立推进，两者共用同一套 surface/damage 传输语义。
>
> 本版已支持**多 surface**：每个 toplevel 一个宿主窗口，surface.create /
> frame / destroy 按 id 路由。宿主已接进 Tauri：`窗口`页扫描 art compositor
> 并开独立窗口，帧走 Tauri 事件、输入走 `surface_input` 命令。
> 当前 macOS 默认 **MTKView/Metal** 原生叠加层，非 macOS 或
> `MIXUM_NATIVE_METAL=0` 回退 WebView canvas；输入/IME/剪贴板仍由
> WebView 处理。XWayland 已接入（toplevel/override-redirect/输入/双向
> 剪贴板）；X11 XIM 输入法与 GPU/dmabuf 零拷贝仍未接入。

## 1. 目标

让容器里的应用像本机应用一样出现在宿主 MixumART：

- 每个 toplevel（后续含 subsurface/popup）一个宿主原生窗口；
- 只传 **buffer + damage**，不传视频；静态/文字区域像素级保真；
- 输入（指针/键盘/触摸/IME/剪贴板）从宿主回传到容器内的 seat；
- 保留 mixumOS 的会话、令牌、接管（agent 持有 / 用户接管）语义；
- 为云端预留同一契约：本地走 vsock/共享内存，远端走压缩/按需回退视频。

## 2. 为什么是“合成器级转发”，不是客户端协议代理

Wayland 客户端把 buffer 交给合成器。B 有两条可能的路：

| 方案 | 谁做合成器 | 转发内容 | 评价 |
|------|-----------|----------|------|
| **B1（推荐）** | 容器内 MixumART 合成器 | 每个 surface 的 buffer + damage + 元数据 | 兼容 XWayland、快照、任意客户端；宿主只需渲染 |
| B2 | 宿主 MixumART | 客户端 `wl_display` 协议 + fd | 最“原生”，但 macOS 宿主没有 Wayland 合成器，fd/dmabuf 跨 VM 极难 |

B1 的关键点：**客户端仍然连接一个真正的合成器**（容器内 MixumART），
所以不需要 LD_PRELOAD、Wayland proxy、waypipe，也不挑客户端。宿主拿到的是
合成器已经规范化后的 surface，而不是客户端私有协议。

## 3. 进程与模块

```
容器内                                    宿主
┌─────────────────────────────┐          ┌──────────────────────────────┐
│ Chromium / foot / XWayland  │          │ MixumART (Tauri)             │
│        │ wl_display          │          │  ├─ 原生窗口（每 surface）    │
│        ▼                     │  surface │  ├─ wgpu/Metal 纹理合成       │
│ mixum-env-compositor         │ ───────▶ │  └─ mixum-env-display        │
│  (Smithay headless)          │  input   │      (SurfaceTransport 客户端)│
│  ├─ seat / xdg-shell         │ ◀─────── │                              │
│  ├─ damage tracker           │          │                              │
│  └─ SurfaceTransport 服务端   │          │                              │
└─────────────────────────────┘          └──────────────────────────────┘
```

新增 crate（建议）：

- `crates/mixum-env-compositor/`（容器内，Linux）：Smithay headless 合成器 +
  `SurfaceTransport` 服务端；输出 surface/damage/seat 事件。
- `crates/mixum-env-display/`（宿主，跨平台）：`SurfaceTransport` 客户端 +
  surface 树模型；纯数据层，不依赖 Tauri。
- `apps/desktop/src-tauri/`：把 `mixum-env-display` 的 surface 映射成原生窗口，
  用 wgpu 渲染纹理；命令层只做装配。
- 传输层可先复用 mixumOS 的 WSS/MAP（令牌、心跳、接管），也可以在同一
  进程内走 Unix socket/vsock；契约不变。

## 4. 传输契约（草案）

### 4.1 surface 元数据

```json
{"type":"surface.create","id":7,"appId":"chromium","title":"…",
 "width":1000,"height":700,"role":"toplevel","parent":null}
{"type":"surface.update","id":7,"title":"…","width":1000,"height":700}
{"type":"surface.destroy","id":7}
{"type":"surface.mode","id":7,"mode":"agent|user|free"}
```

### 4.2 buffer / damage

复用 A 的补丁语义：首帧全量，之后只发脏区；payload 为 PNG（无损）或
按需 JPEG/H.264（动态内容/广域网）。区别只在 surface 维度：

```
surface.frame {
  surface, fullW, fullH,
  x, y, w, h,          // 脏区
  format: "png"|"jpeg"|"h264",
  flags:  keyframe|cursor|patch,
  data
}
```

- 宿主维护每个 surface 的 base texture，只上传 patch 区域；
- 无变化不发帧；尺寸变化发全量；
- `format` 可按订阅者/网络自适应；本地默认 PNG。

### 4.3 输入与 seat

```json
{"type":"seat.pointer","surface":7,"x":120,"y":80,"action":"motion"}
{"type":"seat.button","surface":7,"button":"left","action":"press"}
{"type":"seat.key","surface":7,"key":"Return","mods":["shift"]}
{"type":"seat.focus","surface":7}
```

- 输入事件由宿主窗口捕获，经 `mixum-env-display` 发回容器内 compositor；
- compositor 用真实 `wl_seat` 路由到客户端，保留焦点、grab、IME；
- 接管状态（agent/user/free）由 MAP 的 display.takeover 语义扩展，
  宿主在 `mode=agent` 时只读。

### 4.4 会话与安全

- 复用 MAP `session.hello`/令牌；宿主只连回环或用户显式配置的节点；
- 每个 surface 绑定会话与节点，断线重连走 `session.resume`（surface 状态
  重建失败时回退全量）；
- 宿主不保存应用私有数据；明文 buffer 只在内存中短暂存在。

## 4.5 B0/B1 实现（当前）

新增三个 crate：

| crate | 角色 |
|-------|------|
| `crates/mixum-env-surface` | 共享 wire 协议：surface 元数据、frame/patch、input |
| `crates/mixum-env-compositor` | 容器内 Smithay headless 合成器（Linux） |
| `crates/mixum-env-display` | 宿主 `mixum-art-viewer`（minifb 窗口 + 输入，直连 TCP 调试） |
| `crates/mixum-env-map` | 宿主 MAP WSS 客户端（TOFU + display.list/attach/input/clipboard） |
| `apps/desktop/src-tauri/src/macos` | macOS MTKView/Metal 原生 surface 叠加层；WebView 负责输入 |

当前 wire：

```text
[M A R T][version=1][type][payload_len u32]
  1 = surface.create (JSON: id/appId/title/width/height)
  2 = frame         (33 字节头 + PNG)
  3 = input         (JSON InputEvent)
```

frame 头：`surface, full_w, full_h, x, y, w, h`（7×u32）、
`format u8`（0=仅光标、2=PNG）、`data_len u32`。首帧/尺寸变化为全量，
之后只发脏区；静止不发帧。

运行（Tauri 走 MAP，推荐）：

```sh
# 宿主（Tauri 应用）
cd apps/desktop && npm run tauri dev
# 「窗口」页填入：
#   MAP 地址  wss://<node>:<port>/ws（或 host:port，客户端补 /ws）
#   令牌      map1.…
#   指纹      可选；留空走 TOFU，首次连接会记录到 ~/.mixum/known-hosts

# 容器内：mixcomp（A）和 B compositor 可并存，agentd 按窗口 owner 路由
XDG_RUNTIME_DIR=/run/mixum mixum-env-compositor \
  --map-socket /run/mixum/display.sock
XDG_RUNTIME_DIR=/run/mixum WAYLAND_DISPLAY=mixum-art-0 foot
```

直连 TCP 仍保留给独立 minifb 调试查看器：

```sh
cargo run -p mixum-env-display --release -- --addr <container-ip>:7603
XDG_RUNTIME_DIR=/run/mixum mixum-env-compositor   # 不带 --map-socket
XDG_RUNTIME_DIR=/run/mixum WAYLAND_DISPLAY=mixum-art-0 foot
```

MAP 模式实测：`display.list` 看到 B surface（id 从 1000 起），attach 后
`stream.start` 触发最后一帧全量重放，之后只收脏区补丁；键盘输入经
`display.input` → agentd → B compositor seat → foot，补丁里出现
`got:`/`a`。

A/B 并存实测：agentd 多 comp 源下，`display.list` 同时有 mixcomp 的
id=1（300×169）和 B 的 id=1000（996×663）；attach 后 A 收 5.3KB 全量、
B 收 17.6KB 全量 + 补丁，互不串流。

> agentd 现在支持多个 comp 连接：mixcomp（A）与 B compositor 可并存，
> 窗口按 owner 路由 `stream.start/stop` 与 `display.input`，帧只接受 owner。
> Tauri 已切到 MAP（`mixum-env-map`）；macOS 已有 MTKView/Metal 原生叠加层，
> 非 macOS 或 `MIXUM_NATIVE_METAL=0` 时回退 WebView canvas。
>
> **B2 远端剪贴板（拉取）**：`Cmd/Ctrl+Alt+V` 通过 MAP
> `display.clipboard` 拉取 B compositor 当前 selection 并写入宿主剪贴板；
> host→remote 仍是 `Cmd/Ctrl+V` 粘贴（ASCII）。
>
> **B3 XWayland / A 回退**：B compositor 现在通过 Smithay XWayland + XWM
> 支持 X11 toplevel、override-redirect 菜单、键鼠和 X11↔Wayland 剪贴板。
> 需要镜像内 `xorg-xwayland`；X11 应用用 XWayland 的 `DISPLAY` 启动。
> X11 XIM 输入法尚未接。agentd 多 comp 下仍可用 A 窗口手动回退。

实测（0.1.2 节点，foot 996×663）：全量 17KB，时钟补丁 48×26 约 1.5KB；
TCP 注入 `a` + `Return` 后 foot shell 输出 `got:`/`a`。宿主查看器同时
负责把鼠标/键盘事件写回同一连接。多窗口实测：`art-a`/`art-b` 两个
surface 各自一个窗口，关闭 `art-b` 后收到 `surface.destroy` 且 `art-a`
继续更新。

## 5. 里程碑

| 阶段 | 内容 | 完成标准 |
|------|------|----------|
| **B0 ✅** | `mixum-env-compositor` Smithay headless + 单 toplevel surface 转发；宿主单窗口 PNG 全量 | 容器内 foot/Chromium 起窗，宿主查看器显示 |
| **B1 ✅** | damage 补丁、焦点/指针/键盘；多窗口；Tauri + MAP | 每个 surface 独立窗口；Tauri 通过 MAP WSS 扫描/打开/输入 |
| **B2 ✅（终端 IME 待补）** | subsurface/popup、输入、剪贴板、text-input IME | 弹窗/输入/剪贴板可用；foot 等终端的 IME 仍有限制 |
| **B3 ✅（X11 XIM/拖拽待补）** | XWayland、A 回退、拖拽 | X11 toplevel/菜单/键鼠/剪贴板可用；XIM 未接 |
| **B4** | GPU/dmabuf 零拷贝、远端自适应（视频回退） | 等 virtio-gpu/venus、宿主 GPU 导入成熟 |

GPU 零拷贝（dmabuf/venus/SR-IOV）**等待成熟**：Apple Container/VM 目前
没有稳定的 GPU 导出/导入通路，B0–B3 全部走 CPU `wl_shm` + damage/PNG；
接口保留 `dmabuf` 分支，不阻塞主线。

## 6. 与 A 的关系

| | A（mixumOS / mixcomp） | B（MixumART compositor） |
|---|---|---|
| 生产者 | mixcomp 合成后捕获 | MixumART compositor 每 surface |
| 宿主看到 | 一个窗口一张图 | surface 树 + 原生窗口管理 |
| 输入 | `display.input` → 虚拟设备 | seat 事件 → 真实 wl_seat |
| 兼容性 | 任意 Wayland/X11/XWayland | B1 起 Wayland；XWayland 见 B3 |
| 快照/迁移 | 无状态流，友好 | 连接/fd 绑定，需额外恢复逻辑 |
| 定位 | 立即可用、广覆盖 | 原生体验、长期主线 |

两个生产者共用 `surface.frame` 的 damage/format/input 契约；宿主
`mixum-env-display` 只认契约，不关心帧来自 A 还是 B。A 也是 B 的
回退路径（XWayland、异常应用、GPU 不可用）。

## 7. 非目标（v0）

- 多用户/多租户 surface 隔离；
- 跨宿主迁移活动 surface（快照恢复后重新协商）；
- 在 macOS 上实现完整 Wayland 协议代理（B2）；
- 在没有 GPU 通路的平台上做 dmabuf 零拷贝。
