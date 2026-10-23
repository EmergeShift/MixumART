> English translation of the Chinese document. Original: [display.zh-CN.md](display.zh-CN.md). Technical review pending.

# Display and the Wayland compositor (Route B, design/implementation)

> Status: **B0/B1/B2 implemented** (2026-10-20). The Smithay compositor inside
> the container forwards the `wl_shm` buffer + damage of toplevels and
> subsurface/popups to the host; pointer/keyboard input goes back to the seat
> over MAP, and text-input IME plus clipboard are wired up. Route A (mixcomp
> capture + damage patches + PNG) proceeds independently on the mixumOS side;
> both share the same surface/damage transport semantics.
>
> This version supports **multiple surfaces**: one host window per toplevel, with
> surface.create / frame / destroy routed by id. The host is wired into Tauri:
> the `窗口` page scans art compositors and opens a separate window; frames go
> through Tauri events and input goes through the `surface_input` command.
> On macOS the current default is a native **MTKView/Metal** overlay layer; on
> non-macOS or with `MIXUM_NATIVE_METAL=0` it falls back to WebView canvas, and
> input/IME/clipboard are still handled by the WebView. XWayland is integrated
> (toplevel/override-redirect/input/two-way clipboard); X11 text has a built-in
> XIM server (`XMODIFIERS=@im=mixum`), falling back to XTEST + a temporary keymap
> when XIM is absent; host text/URI/text-file contents can be dropped onto a
> surface. True cross-VM XDND/data-source file drag-and-drop and GPU/dmabuf
> zero-copy are still not integrated.

## 1. Goals

Make applications inside the container appear in the host MixumART like native applications:

- One native host window per toplevel (later including subsurface/popup);
- Transfer only **buffer + damage**, never video; static/text regions stay pixel-exact;
- Route input (pointer/keyboard/touch/IME/clipboard) from the host back to the seat inside the container;
- Preserve mixumOS session, token, and takeover (agent holds / user takes over) semantics;
- Reserve the same contract for the cloud: local goes over vsock/shared memory, remote falls back to compressed/on-demand video.

## 2. Why "compositor-level forwarding" instead of a client protocol proxy

Wayland clients hand their buffers to the compositor. B has two possible routes:

| Approach | Who runs the compositor | What is forwarded | Assessment |
|------|-----------|----------|------|
| **B1 (recommended)** | MixumART compositor inside the container | Per-surface buffer + damage + metadata | Compatible with XWayland, snapshots, and arbitrary clients; the host only renders |
| B2 | Host MixumART | Client `wl_display` protocol + fd | The most "native", but a macOS host has no Wayland compositor, and fd/dmabuf across a VM is extremely hard |

The key point of B1: **the client still connects to a real compositor** (MixumART
inside the container), so no LD_PRELOAD, Wayland proxy, or waypipe is needed, and
no client requires special handling. What the host receives is the surface as the
compositor has already normalized it, not the client's private protocol.

## 3. Processes and modules

> **Transport clarification (2026-10).** The production path is compositor → agentd's
> `/run/mixum/display.sock` → MAP over WSS → host `mixum-env-map`. The
> "surface" in the diagram below is the logical contract; direct TCP is used only for
> `mixum-art-viewer` debugging, not as a second production channel.



```
Container                                Host
┌─────────────────────────────┐          ┌────────────────────────────────┐
│ Chromium / foot / XWayland  │          │ MixumART (Tauri)               │
│        │ wl_display         │          │  ├─ native window (per surface)│
│        ▼                    │  surface │  ├─ wgpu/Metal composition     │
│ mixum-env-compositor        │ ───────▶ │  └─ mixum-env-display          │
│  (Smithay headless)         │  input   │      (SurfaceTransport client) │
│  ├─ seat / xdg-shell        │ ◀─────── │                                │
│  ├─ damage tracker          │          │                                │
│  └─ SurfaceTransport server │          │                                │
└─────────────────────────────┘          └────────────────────────────────┘
```

New crates (proposed):

- `crates/mixum-env-compositor/` (in-container, Linux): Smithay headless compositor +
  `SurfaceTransport` server; emits surface/damage/seat events.
- `crates/mixum-env-display/` (host, cross-platform): `SurfaceTransport` client +
  surface tree model; a pure data layer that does not depend on Tauri.
- `apps/desktop/src-tauri/`: maps `mixum-env-display` surfaces onto native windows and
  renders textures with wgpu; the command layer only does assembly.
- The transport layer can initially reuse mixumOS WSS/MAP (tokens, heartbeats, takeover), or
  run over a Unix socket/vsock in the same process; the contract is unchanged.

## 4. Transport contract (draft)

### 4.1 surface metadata

```json
{"type":"surface.create","id":7,"appId":"chromium","title":"…",
 "width":1000,"height":700,"role":"toplevel","parent":null}
{"type":"surface.update","id":7,"title":"…","width":1000,"height":700}
{"type":"surface.destroy","id":7}
{"type":"surface.mode","id":7,"mode":"agent|user|free"}
```

### 4.2 buffer / damage

Reuse A's patch semantics: the first frame is full, then only dirty regions are sent; the
payload is PNG (lossless) or, on demand, JPEG/H.264 (dynamic content/wide area networks).
The only difference is the surface dimension:

```
surface.frame {
  surface, fullW, fullH,
  x, y, w, h,          // dirty region
  format: "png"|"jpeg"|"h264",
  flags:  keyframe|cursor|patch,
  data
}
```

- The host keeps a base texture per surface and uploads only the patch region;
- No frame is sent when nothing changed; a size change sends a full frame;
- `format` can adapt per subscriber/network; local defaults to PNG.

### 4.3 Input and seat

```json
{"type":"seat.pointer","surface":7,"x":120,"y":80,"action":"motion"}
{"type":"seat.button","surface":7,"button":"left","action":"press"}
{"type":"seat.key","surface":7,"key":"Return","mods":["shift"]}
{"type":"seat.focus","surface":7}
```

- Input events are captured by the host window and sent back through `mixum-env-display`
  to the compositor inside the container;
- The compositor routes them to the client through a real `wl_seat`, preserving focus, grab, and IME;
- Takeover state (agent/user/free) extends MAP's display.takeover semantics;
  the host is read-only when `mode=agent`.

### 4.4 Sessions and security

- Reuse MAP `session.hello`/tokens; the host connects only to the loopback or an explicitly configured node;
- Every surface is bound to a session and a node; reconnection goes through `session.resume`
  (falling back to a full frame when surface state cannot be rebuilt);
- The host does not store application-private data; plaintext buffers exist only briefly in memory.

## 4.5 B0/B1 implementation (current)

Three new crates:

| crate | role |
|-------|------|
| `crates/mixum-env-surface` | Shared wire protocol: surface metadata, frame/patch, input |
| `crates/mixum-env-compositor` | In-container Smithay headless compositor (Linux) |
| `crates/mixum-env-display` | Host `mixum-art-viewer` (minifb window + input, direct TCP debugging) |
| `crates/mixum-env-map` | Host MAP WSS client (TOFU + display.list/attach/input/clipboard) |
| `apps/desktop/src-tauri/src/macos` | macOS MTKView/Metal native surface overlay layer; the WebView handles input |

Current wire:

```text
[M A R T][version=1][type][payload_len u32]
  1 = surface.create (JSON: id/appId/title/width/height)
  2 = frame         (33-byte header + PNG)
  3 = input         (JSON InputEvent)
```

frame header: `surface, full_w, full_h, x, y, w, h` (7×u32),
`format u8` (0=cursor only, 2=PNG), `data_len u32`. The first frame/size change is full,
afterwards only dirty regions are sent; nothing is sent while idle.

### 4.6 Per-frame cost (implementation notes)

A frame has a fixed cost both inside the container and on the host; when changing things, look at them in this order:

| Stage | Location | Approach |
|------|------|------|
| Fetch buffer | `pixels::bgra_to_rgba` | `wl_shm` BGRA/BGRX is converted per row to RGBA8888; `Xrgb8888` is simply recorded as `opaque` |
| Composite | `pixels::blit_rgba` | Clip to the canvas first; when a row is opaque or fully alpha=255, memcpy the whole row, and only genuinely semi-transparent rows are blended per pixel |
| Dirty region | `pixels::diff_bbox` | Run `memcmp` row by row first; only changed rows are searched for the first and last differing pixel |
| Encode | `pixels::encode_png` | Patches use `Fast` + `NoFilter`; full frames use `Fast` + `Sub` (full frames only appear on attach/resize) |

Host side:

- When the macOS native Metal overlay layer takes over rendering, it **no longer** sends
  `surface-frame` to the WebView, avoiding decoding the same frame twice; only
  `MIXUM_NATIVE_METAL=0` or non-macOS goes through canvas.
- PNG in frames sent to the WebView uses base64 (`SurfaceFrameEvent`). Tauri events are
  JSON, so `Vec<u8>` would be serialized as an array of numbers — about 4× the size, and
  the JS side would still have to parse it element by element; base64 is only 1.33× and
  decoding is a single linear scan.
- Metal textures only `replaceRegion` the dirty region instead of re-uploading the whole texture.

`pixels` is pure buffer arithmetic and does not depend on Wayland, so tests and benchmarks can run on the host machine:

```sh
cargo test -p mixum-env-compositor
cargo test -p mixum-env-compositor --release -- --ignored --nocapture   # print timings
```

Running (Tauri over MAP, recommended):

```sh
# Host (Tauri app)
cd apps/desktop && npm run tauri dev
# On the 「窗口」 page fill in:
#   MAP address  wss://<node>:<port>/ws (or host:port; the client appends /ws)
#   Token        map1.…
#   Fingerprint  optional; leave blank for TOFU, the first connection is recorded to ~/.mixum/known-hosts

# In the container: mixcomp (A) and the B compositor can coexist; agentd routes by window owner
XDG_RUNTIME_DIR=/run/mixum mixum-env-compositor \
  --map-socket /run/mixum/display.sock
XDG_RUNTIME_DIR=/run/mixum WAYLAND_DISPLAY=mixum-art-0 foot
```

Direct TCP is still kept for the standalone minifb debug viewer:

```sh
cargo run -p mixum-env-display --release -- --addr <container-ip>:7603
XDG_RUNTIME_DIR=/run/mixum mixum-env-compositor   # without --map-socket
XDG_RUNTIME_DIR=/run/mixum WAYLAND_DISPLAY=mixum-art-0 foot
```

MAP mode measured: `display.list` shows the B surfaces (ids starting at 1000); after
attach, `stream.start` triggers a full replay of the last frame, then only dirty-region
patches are received; keyboard input goes `display.input` → agentd → B compositor seat →
foot, and `got:`/`a` shows up in the patch.

A/B coexistence measured: with multiple comp sources in agentd, `display.list` shows both
mixcomp's id=1 (300×169) and B's id=1000 (996×663); after attach, A receives a 5.3KB full
frame and B receives a 17.6KB full frame + patches, with no cross-streaming.

> agentd now supports multiple comp connections: mixcomp (A) and the B compositor can
> coexist; windows route `stream.start/stop` and `display.input` by owner, and frames are
> only accepted from the owner. Tauri has switched to MAP (`mixum-env-map`); macOS already
> has the MTKView/Metal native overlay layer, falling back to WebView canvas on non-macOS or
> with `MIXUM_NATIVE_METAL=0`.
>
> **B2 remote clipboard (pull)**: `Cmd/Ctrl+Alt+V` pulls the B compositor's current
> selection over MAP `display.clipboard` and writes it to the host clipboard;
> host→remote is still pasting with `Cmd/Ctrl+V` (ASCII).
>
> **B3 XWayland / A fallback**: the B compositor now supports X11 toplevels,
> override-redirect menus, keyboard and mouse, and X11↔Wayland clipboard through
> Smithay XWayland + XWM. Requires `xorg-xwayland` in the image. X11 applications
> **do not need developers to pass their own arguments**: agentd's
> task/spawn_gui/external launch paths automatically inject `DISPLAY` and
> `XMODIFIERS=@im=mixum` from `/run/mixum/xwayland.display`; interactive login shells are
> injected by `/etc/profile.d/mixum-x11.sh`. Non-login `docker exec` can use `sh -lc`
> or source it yourself. With multiple comps in agentd, A windows can still be used as a
> manual fallback.

Measured (0.1.2 node, foot 996×663): full frame 17KB, clock patch 48×26 about 1.5KB;
after injecting `a` + `Return` over TCP, the foot shell outputs `got:`/`a`. The host viewer
is also responsible for writing mouse/keyboard events back over the same connection.
Multi-window measured: `art-a`/`art-b` each get their own window; after closing `art-b`,
`surface.destroy` is received and `art-a` keeps updating.

## 5. Milestones

| Phase | Content | Completion criteria |
|------|------|----------|
| **B0 ✅** | `mixum-env-compositor` Smithay headless + single toplevel surface forwarding; host single-window full PNG | foot/Chromium open a window in the container and the host viewer displays it |
| **B1 ✅** | damage patches, focus/pointer/keyboard; multiple windows; Tauri + MAP | A separate window per surface; Tauri scans/opens/inputs over MAP WSS |
| **B2 ✅ (terminal IME pending)** | subsurface/popup, input, clipboard, text-input IME | Popups/input/clipboard usable; IME in terminals such as foot is still limited |
| **B3 ✅ (full DND pending)** | XWayland, A fallback, drag and drop | X11 toplevels/menus/keyboard and mouse/clipboard/XIM/Unicode text usable; host text/URI drop usable |
| **B4** | GPU/dmabuf zero-copy, remote adaptivity (video fallback) | Waiting for virtio-gpu/venus and host GPU import to mature |

GPU zero-copy (dmabuf/venus/SR-IOV) **is waiting to mature**: Apple Container/VM currently
has no stable GPU export/import path, so B0–B3 all go through CPU `wl_shm` + damage/PNG;
the interface keeps a `dmabuf` branch that does not block the mainline.

## 6. Relationship to A

| | A (mixumOS / mixcomp) | B (MixumART compositor) |
|---|---|---|
| Producer | Captured after mixcomp composites | MixumART compositor, per surface |
| What the host sees | One image in one window | Surface tree + native window management |
| Input | `display.input` → virtual device | seat events → real wl_seat |
| Compatibility | Any Wayland/X11/XWayland | Wayland from B1; XWayland in B3 |
| Snapshot/migration | Stateless stream, friendly | Bound to connection/fd, needs extra recovery logic |
| Positioning | Immediately usable, broad coverage | Native experience, long-term mainline |

Both producers share the damage/format/input contract of `surface.frame`; the host
`mixum-env-display` only honors the contract and does not care whether frames come from A or B. A is also B's
fallback path (XWayland, misbehaving applications, GPU unavailable).

## 7. Non-goals (v0)

- Multi-user/multi-tenant surface isolation;
- Migrating active surfaces across hosts (renegotiate after snapshot restore);
- Implementing a full Wayland protocol proxy on macOS (B2);
- dmabuf zero-copy on platforms without a GPU path.
