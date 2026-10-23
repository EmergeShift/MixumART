# Documentation map

English is the canonical language. Once a page is translated, the English text
lives at `docs/<name>.md` and the Chinese counterpart at
`docs/<name>.zh-CN.md`. Until then the Chinese page is the only text and is
linked below.

## Coverage (2026-10-24)

- English text exists for 10 of 15 pages; 4 of those also have a Chinese
  `.zh-CN.md` counterpart (`architecture`, `cli`, `daemon`, `display`).
- Chinese-first pages still needing an English default: `contributing`,
  `module-map`, `runtime-contract`, `support-matrix`, `ui`.
- English-only pages still needing a Chinese counterpart: `base-env`,
  `browser-bridge`, `browser-early-access`, `criu-build`, `privilege`,
  `snapshot-limits`.

## Pages

| Page | Status | English | 中文 |
|---|---|---|---|
| [architecture](docs/architecture.md) | current | [English](docs/architecture.md) | [中文](docs/architecture.zh-CN.md) |
| [base-env](docs/base-env.md) | current | [English](docs/base-env.md) | pending |
| [browser-bridge](docs/browser-bridge.md) | current | [English](docs/browser-bridge.md) | pending |
| [browser early access](docs/browser-early-access.md) | early access | [English](docs/browser-early-access.md) | pending |
| [CLI](docs/cli.md) | current | [English](docs/cli.md) | [中文](docs/cli.zh-CN.md) |
| [contributing](docs/contributing.md) | current | [English](docs/contributing.md) | [中文](docs/contributing.zh-CN.md) |
| [CRIU build](docs/criu-build.md) | current | [English](docs/criu-build.md) | pending |
| [daemon](docs/daemon.md) | partial | [English](docs/daemon.md) | [中文](docs/daemon.zh-CN.md) |
| [display](docs/display.md) | current (B0/B1/B2) | [English](docs/display.md) | [中文](docs/display.zh-CN.md) |
| [module map](docs/module-map.md) | current | pending | [中文](docs/module-map.md) |
| [privilege](docs/privilege.md) | current | [English](docs/privilege.md) | pending |
| [runtime contract](docs/runtime-contract.md) | current | pending | [中文](docs/runtime-contract.md) |
| [snapshot limits](docs/snapshot-limits.md) | verified | [English](docs/snapshot-limits.md) | pending |
| [support matrix](docs/support-matrix.md) | current | pending | [中文](docs/support-matrix.md) |
| [UI](docs/ui.md) | current | pending | [中文](docs/ui.md) |

## Docs-first / not implemented

- **browser early access** — the extension is not in the Chrome Web Store; it is
  loaded in developer mode. This is an alpha path, not the long-term consumer
  path.
- **mixum-artd** — the daemon exists, but its public command surface is not
  exposed yet.
- **B-route display** — B0/B1/B2 are implemented; the design notes are still
  useful, but treat the platform support matrix as the boundary.
- **CRIU build / snapshot limits** — verified notes, not a promise that every
  host or kernel combination works.

If a page says a feature exists, the code or tests should be the tie-breaker.
