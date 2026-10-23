> English translation of the Chinese document. Original: [ui.zh-CN.md](ui.zh-CN.md). Technical review pending.

# UI structure

The UI is React 19 + TypeScript + Vite with hand-written CSS. No component library.

## 0. Positioning (drives the information architecture)

MART **is not an agent itself** — it was split out of our own agent and open-sourced
for all agents. The UI carries three things:

1. **The human entry point for every agent**: manage and inspect all containers —
   who started each one, which terminal/program it is running, watch and take over
   computer use.
2. **All settings**: mixumOS settings + MART settings.
3. **Agent platform integration**: work mode (tool / sub-agent / isolated
   environment), Agent Core choice (Mixum / Pi / DeepSeek Harness / Codex), BYOK and
   credential migration.

## 1. Window structure

```
┌──────────┬────────────────────────────────────────────────┐
│ ◈ mixum  │  Overview        [demo data] host info  [refresh] │
│   ART    ├────────────────────────────────────────────────┤
│          │  ┌ Desktop entry for all agents ────────────┐  │
│  Overview│  │  description + stats (runtimes/running/owner) │  │
│  Containers│ └──────────────────────────────────────────┘  │
│  Windows │  Running agent environments                     │
│  Agents  │  ┌ Agent card ┐ ┌ Agent card ┐                  │
│  Settings│  │ owner/mode │ │ owner/mode │                  │
│          │  │ thumbnail  │ │ thumbnail  │                  │
│  Runtime │  │ takeover/terminal │ │ takeover/terminal │   │
│  status  │  └────────────┘ └────────────┘                  │
└──────────┴────────────────────────────────────────────────┘
```

- Left rail 224 px: brand, five nav items, runtime status pill, version.
- Top bar: page title + description, demo badge (when opened in a browser), host
  info, refresh.
- Content area scrolls independently; at ≤980 px it degrades (hero stacks, cards
  become one column, top bar wraps).

## 2. View table

| View | File | Data source | Status |
|---|---|---|---|
| Overview | `src/views/Cockpit.tsx` | `list_containers` (aggregates available runtimes) | implemented; environment actions jump to the containers page |
| Containers | `src/views/Containers.tsx` | `list_containers` + connection profiles | implemented: create/start/stop/delete, connect/watch, snapshots, command terminal, logs |
| Windows | `src/views/Surfaces.tsx` / `SurfaceView.tsx` | `list_surfaces` / `open_surface_window` | implemented; takeover via button or Ctrl/Cmd+Shift+T |
| Agents | `src/views/Agents.tsx` | local agent config + system keychain | config persistence implemented; credentials go to the system keychain; external connectors remain the external agent's job |
| Settings | `src/views/Settings.tsx` | `runtime_status` + resource commands | implemented: runtime status, image list, disk usage |

## 3. Data and demo mode

- All invokes go through `src/api.ts`; components never contain command names.
- **Opening in a browser (no Tauri) automatically switches to demo data**
  (`src/demo.ts`) and shows a "demo data" badge — for design review and screenshots,
  never pretending to be real state.
- `Container.agent` is optional: when the backend has no ownership information it
  shows "unlabeled"; it never invents a source.
- Status is queried once on mount and refreshed manually; no background polling.

## 4. Design tokens (warm paper · thin lines · serif headings)

The target is "a desktop tool one person made", not a dashboard template: no
gradients, glow or glass; hierarchy comes from type and lines.

| Variable | Value | Use |
|---|---|---|
| `--paper` | `#f4f2ec` | window background (warm paper) |
| `--surface` / `--surface-2` | `#fffefa` / `#f8f6f0` | panels / table headers and hover |
| `--line` / `--line-strong` | `#e3dfd5` / `#d2cdc0` | dividers / control borders |
| `--ink` / `--ink-2` / `--muted` | `#24221d` / `#4e4a42` / `#7a756c` | body / secondary / helper |
| `--accent` | `#2f5a7a` | selection, focus, links (ink blue) |
| `--ok` / `--warn` / `--danger` | `#2f6b45` / `#8a6320` / `#a4433a` | status colours |
| `--serif` | system serif (Iowan/Songti/Georgia) | headings, statistics |

- Headings use serif, body uses system sans; monospace only for identifiers and
  numbers.
- Focus uses a 2 px outer outline (keyboard reachable); respect
  `prefers-reduced-motion`.
- Relationship to the in-container console: **a counterpart, not reuse** — host is
  warm paper, the container is a light workbench; share state words and monospace
  conventions, not components.

## 5. Copy conventions (for ordinary people)

- Chinese, declarative sentences, no exclamation marks or emoji; **internal
  conventions never appear on screen**.
- State words: `running` / `stopped` / `paused` / `not started` / `unknown`; software
  uses `available` / `not installed` / `not running` / `unavailable`.
- Plain language: "who is using it", "usage mode", "runtime", "environment system",
  "login credentials"; explain a technical term in one sentence the first time it
  appears (e.g. the same-kernel constraint of memory snapshots).
- **The window's A/B channels are a development convention, not exposed to users**:
  A (mixcomp) shows as "compatibility mode", B (compositor) as "enhanced mode", with
  an in-page explanation of each.
- The environment terminal uses the host PTY + runtime `exec` and is a persistent
  interactive session; agent launch is still a one-shot task.
- Technical identifiers (`apple_container`, addresses, paths) use monospace, are not
  translated, and sit in secondary positions.
